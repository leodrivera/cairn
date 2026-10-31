//! Working inside a grant: the file routes under `/api/g/{id}/…`, through the real router.

mod common;

use std::os::unix::fs::symlink;

use axum::http::StatusCode;
use cairn::users::Perms;
use common::{Harness, Req, enc, harness_with_root};
use serde_json::{Value, json};

const PW: &str = "Abcdef1!";
const OWNER: Perms = Perms {
    read: true,
    write: true,
    share: true,
    admin: false,
};
const PLAIN: Perms = Perms {
    read: true,
    write: false,
    share: false,
    admin: false,
};

struct World {
    h: Harness,
    dir: tempfile::TempDir,
    ana: String,
    bob: String,
}

impl World {
    fn root(&self) -> std::path::PathBuf {
        self.dir.path().join("srv")
    }

    fn sql(&self, sql: &str) {
        self.h.state.db.with(|c| c.execute(sql, [])).unwrap();
    }
}

/// Ana (scope `/`) owns `team/{a.txt, sub/b.txt, up -> ../private, home -> ..}`, `private/secret.txt`,
/// `docs/notes.txt` and `read/r.txt`. Bob has scope `/bob` and only `read`.
async fn world(extra: &[(&str, &str)]) -> World {
    let (h, dir) = harness_with_root(extra);
    let root = dir.path().join("srv");
    for (p, t) in [
        ("team/a.txt", "alpha"),
        ("team/sub/b.txt", "beta"),
        ("private/secret.txt", "secret"),
        ("docs/notes.txt", "notes"),
        ("read/r.txt", "r"),
        ("bob/own.txt", "o"),
    ] {
        let f = root.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, t).unwrap();
    }
    symlink("../private", root.join("team/up")).unwrap();
    symlink("..", root.join("team/home")).unwrap();
    h.add_user_scoped("ana@example.com", PW, "/", OWNER);
    h.add_user_scoped("bob@example.com", PW, "/bob", PLAIN);
    let ana = h.login("ana@example.com", PW).await;
    let bob = h.login("bob@example.com", PW).await;
    World { h, dir, ana, bob }
}

async fn give(w: &World, path: &str, permission: &str) -> String {
    let res = Req::post("/api/grants")
        .cookie(&w.ana)
        .json(json!({ "path": path, "emails": ["bob@example.com"], "permission": permission }))
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    res.json["results"][0]["grant"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn names(v: &Value) -> Vec<String> {
    let mut n: Vec<String> = v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();
    n.sort();
    n
}

async fn get(w: &World, c: &str, url: &str) -> common::Res {
    Req::get(url).cookie(c).send(&w.h.app).await
}

async fn del(w: &World, c: &str, url: &str, body: Value) -> common::Res {
    Req::delete(url).cookie(c).json(body).send(&w.h.app).await
}

#[tokio::test]
async fn recipient_reads_folder() {
    let w = world(&[]).await;
    let id = give(&w, "team", "read").await;
    let res = get(&w, &w.bob, &format!("/api/g/{id}/files")).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(names(&res.json), vec!["a.txt", "home", "sub", "up"]);
    let res = get(&w, &w.bob, &format!("/api/g/{id}/files/sub")).await;
    assert_eq!(names(&res.json), vec!["b.txt"]);
    let res = get(&w, &w.bob, &format!("/api/g/{id}/raw/a.txt")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body, b"alpha");
    let res = get(&w, &w.bob, &format!("/api/g/{id}/zip")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("content-type"), "application/zip");
    let res = get(&w, &w.bob, &format!("/api/g/{id}")).await;
    assert_eq!(res.json["grant"]["name"], "team");
    assert_eq!(res.json["grant"]["kind"], "folder");
    assert_eq!(res.json["grant"]["effective"], "read");
}

#[tokio::test]
async fn recipient_writes_folder() {
    let w = world(&[]).await;
    let id = give(&w, "team", "write").await;
    let g = format!("/api/g/{id}");
    let res = Req::put(&format!("{g}/upload/new.txt"))
        .cookie(&w.bob)
        .bytes(b"new")
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let res = Req::post(&format!("{g}/folders/made"))
        .cookie(&w.bob)
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let tag = get(&w, &w.bob, &format!("{g}/raw/a.txt"))
        .await
        .header("etag")
        .to_string();
    let res = Req::put(&format!("{g}/text/a.txt"))
        .cookie(&w.bob)
        .header("if-match", &tag)
        .bytes(b"alpha 2")
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let res = Req::post(&format!("{g}/ops"))
        .cookie(&w.bob)
        .json(json!({ "ops": [
            { "op": "rename", "from": "new.txt", "to": "renamed.txt" },
            { "op": "move", "from": "renamed.txt", "to": "sub/renamed.txt" },
            { "op": "copy", "from": "a.txt", "to": "made/a.txt" }
        ] }))
        .send(&w.h.app)
        .await;
    assert_eq!(res.json["done"], 3, "{}", res.json);
    let root = w.root();
    assert_eq!(
        std::fs::read_to_string(root.join("team/a.txt")).unwrap(),
        "alpha 2"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("team/sub/renamed.txt")).unwrap(),
        "new"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("team/made/a.txt")).unwrap(),
        "alpha 2"
    );
}

#[tokio::test]
async fn read_grant_cannot_write() {
    let w = world(&[]).await;
    let id = give(&w, "team", "read").await;
    let g = format!("/api/g/{id}");
    let tries = [
        Req::put(&format!("{g}/upload/new.txt")).bytes(b"x"),
        Req::post(&format!("{g}/folders/made")),
        Req::put(&format!("{g}/text/a.txt"))
            .header("if-none-match", "*")
            .bytes(b"x"),
        Req::post(&format!("{g}/ops"))
            .json(json!({ "ops": [{ "op": "rename", "from": "a.txt", "to": "b.txt" }] })),
        Req::delete(&format!("{g}/files")).json(json!({ "paths": ["a.txt"] })),
    ];
    for r in tries {
        let res = r.cookie(&w.bob).send(&w.h.app).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.json);
    }
    assert!(w.root().join("team/a.txt").exists());
}

#[tokio::test]
async fn sibling_not_reachable() {
    let w = world(&[]).await;
    let id = give(&w, "team", "write").await;
    let g = format!("/api/g/{id}");
    let res = get(
        &w,
        &w.bob,
        &format!("{g}/raw/{}", enc("../private/secret.txt")),
    )
    .await;
    assert_ne!(res.status, StatusCode::OK);
    assert_ne!(res.body, b"secret");
    let res = get(&w, &w.bob, &format!("{g}/raw/up/secret.txt")).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.code(), "outside");
    let res = get(&w, &w.bob, &format!("{g}/files/home")).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.code(), "outside");
    let res = Req::post(&format!("{g}/ops"))
        .cookie(&w.bob)
        .json(json!({ "ops": [{ "op": "copy", "from": "up", "to": "stolen" }] }))
        .send(&w.h.app)
        .await;
    assert!(
        !w.root().join("team/stolen/secret.txt").exists(),
        "{}",
        res.json
    );
}

#[tokio::test]
async fn granted_item_itself_fixed() {
    let w = world(&[]).await;
    let id = give(&w, "team", "write").await;
    let g = format!("/api/g/{id}");
    let res = del(&w, &w.bob, &format!("{g}/files"), json!({ "paths": [""] })).await;
    assert_eq!(res.json["failed"][0]["error"], "scope_root", "{}", res.json);
    let res = Req::post(&format!("{g}/ops"))
        .cookie(&w.bob)
        .json(json!({ "ops": [{ "op": "rename", "from": "", "to": "x" }] }))
        .send(&w.h.app)
        .await;
    assert_eq!(res.json["failed"][0]["error"], "scope_root", "{}", res.json);
    assert!(w.root().join("team/a.txt").exists());
}

#[tokio::test]
async fn file_grant_contents_only() {
    let w = world(&[]).await;
    let id = give(&w, "docs/notes.txt", "write").await;
    let g = format!("/api/g/{id}");
    let res = get(&w, &w.bob, &format!("{g}/raw")).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.body, b"notes");
    let tag = res.header("etag").to_string();
    let res = Req::put(&format!("{g}/text"))
        .cookie(&w.bob)
        .header("if-match", &tag)
        .bytes(b"notes 2")
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let res = Req::put(&format!("{g}/upload?replace=1"))
        .cookie(&w.bob)
        .bytes(b"notes 3")
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(
        std::fs::read_to_string(w.root().join("docs/notes.txt")).unwrap(),
        "notes 3"
    );
    assert_eq!(
        get(&w, &w.bob, &format!("{g}/files")).await.status,
        StatusCode::FORBIDDEN
    );
    let res = get(&w, &w.bob, &format!("{g}/raw/other.txt")).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.code(), "grant_gone");
    let res = del(&w, &w.bob, &format!("{g}/files"), json!({ "paths": [""] })).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = Req::put(&format!("{g}/upload"))
        .cookie(&w.bob)
        .bytes(b"x")
        .send(&w.h.app)
        .await;
    assert_ne!(res.status, StatusCode::CREATED);
    assert!(w.root().join("docs/notes.txt").exists());
    let res = get(&w, &w.bob, &format!("/api/g/{id}")).await;
    assert_eq!(res.json["grant"]["kind"], "file");
    assert_eq!(res.json["grant"]["name"], "notes.txt");
}

async fn gone(w: &World, id: &str) {
    let res = get(w, &w.bob, &format!("/api/g/{id}/files")).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.json);
    assert_eq!(res.code(), "grant_gone");
    let res = get(w, &w.bob, "/api/grants/received").await;
    assert_eq!(res.json["grants"], json!([]));
}

async fn works(w: &World, id: &str) {
    let res = get(w, &w.bob, &format!("/api/g/{id}/files")).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
}

#[tokio::test]
async fn grant_dies_in_each_case() {
    for case in [
        "expired",
        "revoked",
        "no_share",
        "scope_moved",
        "deleted",
        "renamed",
    ] {
        let w = world(&[]).await;
        let id = give(&w, "team", "read").await;
        works(&w, &id).await;
        match case {
            "expired" => w.sql("UPDATE grants SET expires_at = 1"),
            "revoked" => {
                Req::delete(&format!("/api/grants/{id}"))
                    .cookie(&w.ana)
                    .send(&w.h.app)
                    .await;
            }
            "no_share" => w.sql("UPDATE users SET perm_share = 0 WHERE email = 'ana@example.com'"),
            "scope_moved" => {
                w.sql("UPDATE users SET scope = '/private' WHERE email = 'ana@example.com'")
            }
            "deleted" => std::fs::remove_dir_all(w.root().join("team")).unwrap(),
            _ => std::fs::rename(w.root().join("team"), w.root().join("team-2")).unwrap(),
        }
        gone(&w, &id).await;
    }
}

#[tokio::test]
async fn grant_comes_back() {
    let w = world(&[]).await;
    let id = give(&w, "team", "read").await;
    w.sql("UPDATE users SET perm_share = 0 WHERE email = 'ana@example.com'");
    gone(&w, &id).await;
    w.sql("UPDATE users SET perm_share = 1 WHERE email = 'ana@example.com'");
    works(&w, &id).await;
    w.sql("UPDATE users SET scope = '/private' WHERE email = 'ana@example.com'");
    gone(&w, &id).await;
    w.sql("UPDATE users SET scope = '/' WHERE email = 'ana@example.com'");
    works(&w, &id).await;
    std::fs::rename(w.root().join("team"), w.root().join("team-2")).unwrap();
    gone(&w, &id).await;
    std::fs::rename(w.root().join("team-2"), w.root().join("team")).unwrap();
    works(&w, &id).await;
}

#[tokio::test]
async fn owner_loses_write_grant_reads() {
    let w = world(&[]).await;
    let id = give(&w, "team", "write").await;
    w.sql("UPDATE users SET perm_write = 0 WHERE email = 'ana@example.com'");
    let up = || {
        Req::put(&format!("/api/g/{id}/upload/n.txt"))
            .cookie(&w.bob)
            .bytes(b"n")
    };
    assert_eq!(up().send(&w.h.app).await.status, StatusCode::FORBIDDEN);
    works(&w, &id).await;
    let res = get(&w, &w.bob, "/api/grants/received").await;
    assert_eq!(res.json["grants"][0]["permission"], "write");
    assert_eq!(res.json["grants"][0]["effective"], "read");
    w.sql("UPDATE users SET perm_write = 1 WHERE email = 'ana@example.com'");
    assert_eq!(up().send(&w.h.app).await.status, StatusCode::CREATED);
}

#[tokio::test]
async fn no_reshare() {
    let w = world(&[]).await;
    w.sql("UPDATE users SET perm_share = 1 WHERE email = 'bob@example.com'");
    let id = give(&w, "team", "write").await;
    let res = Req::post(&format!("/api/g/{id}/shares"))
        .cookie(&w.bob)
        .json(json!({ "path": "a.txt", "expiresDays": 7, "download": true }))
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = Req::post(&format!("/api/g/{id}/grants"))
        .cookie(&w.bob)
        .json(json!({ "path": "a.txt", "emails": ["ana@example.com"], "permission": "read" }))
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = Req::post("/api/shares")
        .cookie(&w.bob)
        .json(json!({ "path": "team/a.txt", "expiresDays": 7, "download": true }))
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let shares: i64 =
        w.h.state
            .db
            .with(|c| c.query_row("SELECT count(*) FROM shares", [], |r| r.get(0)))
            .unwrap();
    assert_eq!(shares, 0);
}

async fn trash_of(w: &World, c: &str) -> Vec<Value> {
    get(w, c, "/api/trash").await.json["entries"]
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn recipient_delete_goes_to_trash() {
    let w = world(&[]).await;
    let id = give(&w, "team", "write").await;
    let res = del(
        &w,
        &w.bob,
        &format!("/api/g/{id}/files"),
        json!({ "paths": ["a.txt"] }),
    )
    .await;
    assert_eq!(res.json["deleted"], 1, "{}", res.json);
    assert!(!w.root().join("team/a.txt").exists());
    let e = &trash_of(&w, &w.ana).await[0];
    assert_eq!(e["name"], "a.txt");
    assert_eq!(e["deletedBy"]["name"], "Someone");
    let bob_id: i64 =
        w.h.state
            .db
            .with(|c| {
                c.query_row(
                    "SELECT id FROM users WHERE email = 'bob@example.com'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
    assert_eq!(e["deletedBy"]["id"], bob_id);
}

#[tokio::test]
async fn recipient_permanent_refused() {
    let w = world(&[]).await;
    let id = give(&w, "team", "write").await;
    let res = del(
        &w,
        &w.bob,
        &format!("/api/g/{id}/files"),
        json!({ "paths": ["a.txt"], "permanent": true }),
    )
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.json);
    assert!(w.root().join("team/a.txt").exists());
}

#[tokio::test]
async fn recipient_delete_refused_when_trash_off() {
    let w = world(&[("CAIRN_TRASH_DAYS", "0")]).await;
    let id = give(&w, "team", "write").await;
    let res = del(
        &w,
        &w.bob,
        &format!("/api/g/{id}/files"),
        json!({ "paths": ["a.txt"] }),
    )
    .await;
    assert_eq!(res.json["failed"][0]["error"], "trash_off", "{}", res.json);
    assert!(w.root().join("team/a.txt").exists());
}

#[tokio::test]
async fn recipient_restores_not_deletes() {
    let w = world(&[]).await;
    let id = give(&w, "team", "write").await;
    let res = del(
        &w,
        &w.bob,
        &format!("/api/g/{id}/files"),
        json!({ "paths": ["a.txt", "sub"] }),
    )
    .await;
    let ids: Vec<String> = res.json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().into())
        .collect();
    let mine = trash_of(&w, &w.bob).await;
    assert_eq!(mine.len(), 2);
    assert!(mine.iter().all(|e| e["canDelete"] == false));
    assert_eq!(
        mine.iter().find(|e| e["name"] == "a.txt").unwrap()["from"],
        "/team"
    );
    let res = del(&w, &w.bob, "/api/trash", json!({ "ids": [ids[0]] })).await;
    assert_eq!(res.json["deleted"], 0, "{}", res.json);
    let res = Req::post("/api/trash/empty")
        .cookie(&w.bob)
        .send(&w.h.app)
        .await;
    assert_eq!(res.json["deleted"], 0);
    assert_eq!(trash_of(&w, &w.bob).await.len(), 2);
    let res = Req::post("/api/trash/restore")
        .cookie(&w.bob)
        .json(json!({ "ids": ids }))
        .send(&w.h.app)
        .await;
    assert_eq!(
        res.json["restored"].as_array().unwrap().len(),
        2,
        "{}",
        res.json
    );
    assert_eq!(
        std::fs::read_to_string(w.root().join("team/a.txt")).unwrap(),
        "alpha"
    );
    assert!(w.root().join("team/sub/b.txt").exists());
}

#[tokio::test]
async fn owner_sees_recipient_deletion() {
    let w = world(&[]).await;
    let id = give(&w, "team", "write").await;
    del(
        &w,
        &w.bob,
        &format!("/api/g/{id}/files"),
        json!({ "paths": ["a.txt"] }),
    )
    .await;
    let e = &trash_of(&w, &w.ana).await[0];
    assert_eq!(e["canDelete"], true);
    assert_eq!(e["from"], "/team");
}

#[tokio::test]
async fn trash_visibility_follows_access() {
    let w = world(&[]).await;
    let id = give(&w, "team", "write").await;
    del(
        &w,
        &w.bob,
        &format!("/api/g/{id}/files"),
        json!({ "paths": ["a.txt"] }),
    )
    .await;
    assert_eq!(trash_of(&w, &w.bob).await.len(), 1);
    Req::delete(&format!("/api/grants/{id}"))
        .cookie(&w.ana)
        .send(&w.h.app)
        .await;
    assert!(trash_of(&w, &w.bob).await.is_empty());
    assert_eq!(trash_of(&w, &w.ana).await.len(), 1);
    // A read grant shows nothing either.
    give(&w, "team", "read").await;
    assert!(trash_of(&w, &w.bob).await.is_empty());
}

#[tokio::test]
async fn grant_returns_after_restore() {
    let w = world(&[]).await;
    let id = give(&w, "team", "read").await;
    let res = del(&w, &w.ana, "/api/files", json!({ "paths": ["team"] })).await;
    let entry = res.json["entries"][0].as_str().unwrap().to_string();
    gone(&w, &id).await;
    let res = Req::post("/api/trash/restore")
        .cookie(&w.ana)
        .json(json!({ "ids": [entry] }))
        .send(&w.h.app)
        .await;
    assert_eq!(res.json["restored"][0]["path"], "team", "{}", res.json);
    works(&w, &id).await;
    assert_eq!(
        get(&w, &w.bob, "/api/grants/received").await.json["grants"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn grant_of_root_hides_trash() {
    let w = world(&[]).await;
    let id = give(&w, "", "write").await;
    del(
        &w,
        &w.bob,
        &format!("/api/g/{id}/files"),
        json!({ "paths": ["team/a.txt"] }),
    )
    .await;
    assert!(w.root().join(".cairn-trash").exists());
    let res = get(&w, &w.bob, &format!("/api/g/{id}/files")).await;
    assert!(
        !names(&res.json).contains(&".cairn-trash".to_string()),
        "{}",
        res.json
    );
    let res = get(&w, &w.bob, &format!("/api/g/{id}/files/.cairn-trash")).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn zip_of_chosen_items_in_grant() {
    let w = world(&[]).await;
    let id = give(&w, "team", "read").await;
    let res = Req::post(&format!("/api/g/{id}/zip"))
        .no_csrf()
        .cookie(&w.bob)
        .header("sec-fetch-site", "same-origin")
        .header("content-type", "application/x-www-form-urlencoded")
        .bytes(b"item=a.txt&item=sub")
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.header("content-type"), "application/zip");
    assert!(res.header("content-disposition").contains("team.zip"));
    let res = Req::post(&format!("/api/g/{id}/zip"))
        .no_csrf()
        .cookie(&w.bob)
        .header("sec-fetch-site", "cross-site")
        .bytes(b"item=a.txt")
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn grant_paths_decoded_once() {
    use std::os::unix::ffi::OsStrExt;
    let w = world(&[]).await;
    let team = w.root().join("team");
    std::fs::write(team.join("Report #1 (50%) ✓ café.txt"), "unicode").unwrap();
    std::fs::write(team.join("q?r.txt"), "query").unwrap();
    std::fs::write(team.join("x%41.txt"), "percent").unwrap();
    std::fs::write(team.join("xA.txt"), "other").unwrap();
    std::fs::create_dir(team.join("a b")).unwrap();
    std::fs::write(team.join("a b/c.txt"), "space").unwrap();
    let raw: &[u8] = b"caf\xe9.txt";
    std::fs::write(team.join(std::ffi::OsStr::from_bytes(raw)), "raw").unwrap();
    let id = give(&w, "team", "write").await;
    for (name, body) in [
        ("Report #1 (50%) ✓ café.txt", "unicode"),
        ("q?r.txt", "query"),
        ("x%41.txt", "percent"),
        ("a b/c.txt", "space"),
    ] {
        let res = get(&w, &w.bob, &format!("/api/g/{id}/raw/{}", enc(name))).await;
        assert_eq!(res.status, StatusCode::OK, "{name}: {}", res.json);
        assert_eq!(res.body, body.as_bytes(), "{name}");
    }
    let res = get(&w, &w.bob, &format!("/api/g/{id}/files/{}", enc("a b"))).await;
    assert_eq!(names(&res.json), vec!["c.txt"]);
    let seg = cairn::fs::path::RelPath::segment_for(raw);
    let res = get(&w, &w.bob, &format!("/api/g/{id}/raw/{}", enc(&seg))).await;
    assert_eq!(res.status, StatusCode::OK, "raw: {}", res.json);
    assert_eq!(res.body, b"raw");
    // A save to the name with %41 in it changes that file, never xA.txt.
    let url = format!("/api/g/{id}/text/{}", enc("x%41.txt"));
    let raw_url = format!("/api/g/{id}/raw/{}", enc("x%41.txt"));
    let tag = get(&w, &w.bob, &raw_url).await.header("etag").to_string();
    let res = Req::put(&url)
        .cookie(&w.bob)
        .header("if-match", &tag)
        .bytes(b"changed")
        .send(&w.h.app)
        .await;
    assert!(res.status.is_success(), "{}", res.json);
    assert_eq!(std::fs::read(team.join("x%41.txt")).unwrap(), b"changed");
    assert_eq!(std::fs::read(team.join("xA.txt")).unwrap(), b"other");
}
