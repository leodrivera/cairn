//! The trash, through the real router: where deleted items go, and how they come back.

mod common;

use axum::http::StatusCode;
use cairn::users::Perms;
use common::{Harness, Req, enc, harness_with_root};
use serde_json::json;

const PW: &str = "Abcdef1!";
const ALL: Perms = Perms {
    read: true,
    write: true,
    share: true,
    admin: false,
};

/// A user with the whole root as scope, signed in. Returns the cookie and the root on disk.
async fn root_user(h: &Harness, dir: &tempfile::TempDir) -> (String, std::path::PathBuf) {
    h.add_user_scoped("ana@example.com", PW, "/", ALL);
    (h.login("ana@example.com", PW).await, dir.path().join("srv"))
}

#[tokio::test]
async fn me_reports_trash_on() {
    let (h, dir) = harness_with_root(&[]);
    let (c, _) = root_user(&h, &dir).await;
    let res = Req::get("/api/me").cookie(&c).send(&h.app).await;
    assert_eq!(res.json["trash"], json!({ "on": true }));
    let (h, dir) = harness_with_root(&[("CAIRN_TRASH_DAYS", "0")]);
    let (c, _) = root_user(&h, &dir).await;
    let res = Req::get("/api/me").cookie(&c).send(&h.app).await;
    assert_eq!(res.json["trash"], json!({ "on": false }));
}

#[tokio::test]
async fn trash_name_reserved_for_uploads_at_root() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    let res = Req::put(&format!("/api/upload/{}", enc(".cairn-trash")))
        .cookie(&c)
        .bytes(b"x")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.json);
    assert_eq!(res.code(), "reserved");
    assert!(!root.join(".cairn-trash").exists());
    std::fs::create_dir_all(root.join("docs")).unwrap();
    let res = Req::put(&format!("/api/upload/{}", enc("docs/.cairn-trash")))
        .cookie(&c)
        .bytes(b"x")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
}

const READER: Perms = Perms {
    read: true,
    write: false,
    share: false,
    admin: false,
};

async fn delete(h: &Harness, c: &str, paths: &[&str], permanent: Option<bool>) -> common::Res {
    let mut body = json!({ "paths": paths });
    if let Some(p) = permanent {
        body["permanent"] = json!(p);
    }
    Req::delete("/api/files")
        .cookie(c)
        .json(body)
        .send(&h.app)
        .await
}

async fn trash_list(h: &Harness, c: &str) -> Vec<serde_json::Value> {
    let res = Req::get("/api/trash").cookie(c).send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    res.json["entries"].as_array().unwrap().clone()
}

async fn restore(h: &Harness, c: &str, ids: &[&str]) -> common::Res {
    Req::post("/api/trash/restore")
        .cookie(c)
        .json(json!({ "ids": ids }))
        .send(&h.app)
        .await
}

fn put(root: &std::path::Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn rows(h: &Harness) -> i64 {
    h.state
        .db
        .with(|c| c.query_row("SELECT count(*) FROM trash", [], |r| r.get(0)))
        .unwrap()
}

fn ids(res: &common::Res) -> Vec<String> {
    res.json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn delete_moves_to_trash() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "docs/a.txt", "alpha");
    put(&root, "docs/b.txt", "beta");
    let res = delete(&h, &c, &["docs/a.txt"], None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["deleted"], 1);
    let ids = ids(&res);
    assert_eq!(ids.len(), 1);
    assert!(!root.join("docs/a.txt").exists());
    let kept = root.join(".cairn-trash").join(&ids[0]).join("a.txt");
    assert_eq!(std::fs::read_to_string(kept).unwrap(), "alpha");
    assert_eq!(rows(&h), 1);
    // Overwriting is not a delete: nothing more enters the trash.
    let res = Req::put("/api/upload/docs/b.txt?replace=1")
        .cookie(&c)
        .bytes(b"beta 2")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(rows(&h), 1);
}

#[tokio::test]
async fn delete_folder_keeps_tree() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "Archive/2026/notes.txt", "n");
    put(&root, "Archive/readme.md", "r");
    let res = delete(&h, &c, &["Archive"], None).await;
    let id = &ids(&res)[0];
    let kept = root.join(".cairn-trash").join(id).join("Archive");
    assert_eq!(
        std::fs::read_to_string(kept.join("2026/notes.txt")).unwrap(),
        "n"
    );
    assert_eq!(
        std::fs::read_to_string(kept.join("readme.md")).unwrap(),
        "r"
    );
    let list = trash_list(&h, &c).await;
    assert_eq!(list[0]["kind"], "folder");
    assert_eq!(list[0]["size"], 2);
}

#[tokio::test]
async fn permanent_delete_removes() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "a.txt", "a");
    let res = delete(&h, &c, &["a.txt"], Some(true)).await;
    assert_eq!(res.json["deleted"], 1, "{}", res.json);
    assert!(ids(&res).is_empty());
    assert!(!root.join("a.txt").exists());
    assert_eq!(rows(&h), 0);
}

#[tokio::test]
async fn trash_off_refuses_plain_delete() {
    let (h, dir) = harness_with_root(&[("CAIRN_TRASH_DAYS", "0")]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "a.txt", "a");
    let res = delete(&h, &c, &["a.txt"], None).await;
    assert_eq!(res.json["deleted"], 0, "{}", res.json);
    assert_eq!(res.json["failed"][0]["error"], "trash_off");
    assert!(root.join("a.txt").exists());
    let res = delete(&h, &c, &["a.txt"], Some(true)).await;
    assert_eq!(res.json["deleted"], 1, "{}", res.json);
    assert!(!root.join("a.txt").exists());
}

#[tokio::test]
async fn taken_folder_refuses_plain_delete() {
    let dir0 = tempfile::tempdir().unwrap();
    let pre = dir0.path().join("srv/.cairn-trash");
    std::fs::create_dir_all(&pre).unwrap();
    std::fs::write(pre.join("notes.txt"), "mine").unwrap();
    let (h, dir) = common::harness_at(dir0, &[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "a.txt", "a");
    let res = delete(&h, &c, &["a.txt"], None).await;
    assert_eq!(
        res.json["failed"][0]["error"], "trash_taken",
        "{}",
        res.json
    );
    assert_eq!(
        res.json["failed"][0]["message"],
        "The trash for this folder cannot be used: /.cairn-trash was not made by Cairn. Ask an admin to rename it."
    );
    assert!(root.join("a.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.join(".cairn-trash/notes.txt")).unwrap(),
        "mine"
    );
}

#[tokio::test]
async fn delete_in_own_mount_uses_its_trash() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "photos/2026/trail-ridge.jpg", "jpg");
    put(&root, "top.txt", "t");
    use cairn::fs::mount::fake_mount_id;
    fake_mount_id(dir.path(), 9000);
    fake_mount_id(&root, 9001);
    fake_mount_id(&root.join("photos"), 9002);
    fake_mount_id(&root.join("photos/2026"), 9002);
    let res = delete(&h, &c, &["photos/2026/trail-ridge.jpg"], None).await;
    let id = &ids(&res)[0];
    assert!(
        root.join("photos/.cairn-trash")
            .join(id)
            .join("trail-ridge.jpg")
            .exists()
    );
    let dir_of: String = h
        .state
        .db
        .with(|c| {
            c.query_row("SELECT trash_dir FROM trash WHERE id = ?1", [id], |r| {
                r.get(0)
            })
        })
        .unwrap();
    assert_eq!(dir_of, "/photos/.cairn-trash");
    let res = delete(&h, &c, &["top.txt"], None).await;
    assert!(
        root.join(".cairn-trash")
            .join(&ids(&res)[0])
            .join("top.txt")
            .exists()
    );
    // Nobody reaches either trash folder.
    let res = Req::get("/api/files/photos/.cairn-trash")
        .cookie(&c)
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(
        restore(&h, &c, &[id]).await.json["restored"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(root.join("photos/2026/trail-ridge.jpg").exists());
}

#[tokio::test]
async fn other_disk_refused() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "usb/a.txt", "a");
    cairn::trash::ops::fail_rename_for(&root.join("usb/a.txt"));
    let res = delete(&h, &c, &["usb/a.txt"], None).await;
    assert_eq!(res.json["failed"][0]["error"], "other_disk", "{}", res.json);
    assert_eq!(
        res.json["failed"][0]["message"],
        "a.txt is on another disk, so it cannot go to the trash."
    );
    assert!(root.join("usb/a.txt").exists());
    assert_eq!(rows(&h), 0);
    let left: Vec<_> = std::fs::read_dir(root.join(".cairn-trash"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n != ".cairn-trash-marker")
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[tokio::test]
async fn list_shows_entry() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "docs/notes.txt", "12345");
    delete(&h, &c, &["docs/notes.txt"], None).await;
    let e = &trash_list(&h, &c).await[0];
    assert_eq!(e["name"], "notes.txt");
    assert_eq!(e["kind"], "file");
    assert_eq!(e["size"], 5);
    assert_eq!(e["from"], "/docs");
    assert_eq!(e["deletedBy"]["name"], "Someone");
    assert_eq!(e["canDelete"], true);
    assert!(e.get("rawName").is_none());
    let (at, until) = (
        e["deletedAt"].as_i64().unwrap(),
        e["expiresAt"].as_i64().unwrap(),
    );
    assert_eq!(until - at, 30 * 86_400);
}

#[tokio::test]
async fn restore_to_original() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "docs/notes.txt", "keep me");
    let res = delete(&h, &c, &["docs/notes.txt"], None).await;
    let id = ids(&res)[0].clone();
    let res = restore(&h, &c, &[&id]).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["restored"][0]["path"], "docs/notes.txt");
    assert_eq!(
        std::fs::read_to_string(root.join("docs/notes.txt")).unwrap(),
        "keep me"
    );
    assert_eq!(rows(&h), 0);
    assert!(!root.join(".cairn-trash").join(&id).exists());
}

#[tokio::test]
async fn restore_recreates_parents() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "docs/2026/notes.txt", "n");
    let id = ids(&delete(&h, &c, &["docs/2026/notes.txt"], None).await)[0].clone();
    delete(&h, &c, &["docs"], Some(true)).await;
    assert!(!root.join("docs").exists());
    let res = restore(&h, &c, &[&id]).await;
    assert_eq!(
        res.json["restored"][0]["path"], "docs/2026/notes.txt",
        "{}",
        res.json
    );
    assert_eq!(
        std::fs::read_to_string(root.join("docs/2026/notes.txt")).unwrap(),
        "n"
    );
}

#[tokio::test]
async fn restore_taken_name() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    let mut got = Vec::new();
    for _ in 0..3 {
        put(&root, "report.txt", "r");
        got.push(ids(&delete(&h, &c, &["report.txt"], None).await)[0].clone());
    }
    for id in &got {
        restore(&h, &c, &[id]).await;
    }
    for name in [
        "report.txt",
        "report (restored).txt",
        "report (restored 2).txt",
    ] {
        assert!(root.join(name).exists(), "{name}");
    }
    std::fs::create_dir_all(root.join("Archive")).unwrap();
    let id = ids(&delete(&h, &c, &["Archive"], None).await)[0].clone();
    std::fs::create_dir_all(root.join("Archive")).unwrap();
    let res = restore(&h, &c, &[&id]).await;
    assert_eq!(res.json["restored"][0]["path"], "Archive (restored)");
    assert!(root.join("Archive (restored)").is_dir());
}

#[tokio::test]
async fn restore_needs_write() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "a.txt", "a");
    let id = ids(&delete(&h, &c, &["a.txt"], None).await)[0].clone();
    h.add_user_scoped("bob@example.com", PW, "/", READER);
    let b = h.login("bob@example.com", PW).await;
    assert!(trash_list(&h, &b).await.is_empty());
    let res = restore(&h, &b, &[&id]).await;
    assert_eq!(res.json["restored"], json!([]));
    assert_eq!(res.json["failed"][0]["id"], id.as_str());
    assert_eq!(res.json["failed"][0]["error"], "not_found");
    assert!(!root.join("a.txt").exists());
}

#[tokio::test]
async fn permanent_from_trash() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "a.txt", "a");
    let id = ids(&delete(&h, &c, &["a.txt"], None).await)[0].clone();
    let res = Req::delete("/api/trash")
        .cookie(&c)
        .json(json!({ "ids": [id] }))
        .send(&h.app)
        .await;
    assert_eq!(res.json["deleted"], 1, "{}", res.json);
    assert!(!root.join(".cairn-trash").join(&id).exists());
    assert_eq!(rows(&h), 0);
}

#[tokio::test]
async fn empty_trash() {
    let (h, dir) = harness_with_root(&[]);
    let root = dir.path().join("srv");
    put(&root, "a/one.txt", "1");
    put(&root, "b/two.txt", "2");
    h.add_user_scoped("ana@example.com", PW, "/a", ALL);
    h.add_user_scoped("bob@example.com", PW, "/b", ALL);
    let a = h.login("ana@example.com", PW).await;
    let b = h.login("bob@example.com", PW).await;
    delete(&h, &a, &["one.txt"], None).await;
    delete(&h, &b, &["two.txt"], None).await;
    assert_eq!(trash_list(&h, &a).await.len(), 1);
    let res = Req::post("/api/trash/empty").cookie(&a).send(&h.app).await;
    assert_eq!(res.json["deleted"], 1, "{}", res.json);
    assert!(trash_list(&h, &a).await.is_empty());
    assert_eq!(trash_list(&h, &b).await.len(), 1);
}

#[tokio::test]
async fn expiry_fixed_at_delete() {
    let (h, dir) = harness_with_root(&[]);
    let root = dir.path().join("srv");
    h.add_user("admin@example.com", Some(PW), true);
    let c = h.login("admin@example.com", PW).await;
    put(&root, "a.txt", "a");
    delete(&h, &c, &["a.txt"], None).await;
    let before = trash_list(&h, &c).await[0]["expiresAt"].clone();
    Req::put("/api/settings")
        .cookie(&c)
        .json(json!({ "theme": null, "thumbnails": false, "trashDays": 0 }))
        .send(&h.app)
        .await;
    let list = trash_list(&h, &c).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["expiresAt"], before);
    put(&root, "b.txt", "b");
    assert_eq!(
        delete(&h, &c, &["b.txt"], None).await.json["failed"][0]["error"],
        "trash_off"
    );
}

#[tokio::test]
async fn purge_removes_expired() {
    let (h, dir) = harness_with_root(&[]);
    let root = dir.path().join("srv");
    h.add_user("admin@example.com", Some(PW), true);
    let c = h.login("admin@example.com", PW).await;
    put(&root, "old.txt", "o");
    let young = ids(&delete(&h, &c, &["old.txt"], None).await)[0].clone();
    Req::put("/api/settings")
        .cookie(&c)
        .json(json!({ "theme": null, "thumbnails": false, "trashDays": 1 }))
        .send(&h.app)
        .await;
    put(&root, "soon.txt", "s");
    let soon = ids(&delete(&h, &c, &["soon.txt"], None).await)[0].clone();
    let later = cairn::now() + 2 * 86_400;
    assert_eq!(
        cairn::trash::ops::purge(&h.state.trash, &h.state.db, later),
        1
    );
    assert!(!root.join(".cairn-trash").join(&soon).exists());
    assert!(
        root.join(".cairn-trash")
            .join(&young)
            .join("old.txt")
            .exists()
    );
    assert_eq!(rows(&h), 1);
}

#[tokio::test]
async fn reconcile_drops_rows_without_files() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "a.txt", "a");
    let id = ids(&delete(&h, &c, &["a.txt"], None).await)[0].clone();
    std::fs::remove_dir_all(root.join(".cairn-trash").join(&id)).unwrap();
    assert_eq!(
        cairn::trash::ops::reconcile(&h.state.trash, &h.state.db),
        (1, 0)
    );
    assert!(trash_list(&h, &c).await.is_empty());
}

#[tokio::test]
async fn reconcile_leaves_unknown_folders() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "a.txt", "a");
    delete(&h, &c, &["a.txt"], None).await;
    put(&root, ".cairn-trash/zzzz/x.txt", "stranger");
    assert_eq!(
        cairn::trash::ops::reconcile(&h.state.trash, &h.state.db),
        (0, 1)
    );
    cairn::trash::ops::purge(&h.state.trash, &h.state.db, cairn::now() + 365 * 86_400);
    assert_eq!(
        std::fs::read_to_string(root.join(".cairn-trash/zzzz/x.txt")).unwrap(),
        "stranger"
    );
}

#[tokio::test]
async fn trash_size_walk_skips_unreadable() {
    use std::os::unix::fs::MetadataExt;
    if std::fs::metadata("/proc/self").unwrap().uid() == 0 {
        return; // root reads everything
    }
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "big/a.txt", "12345");
    put(&root, "big/locked/secret.txt", "hidden");
    symlink("/etc", root.join("big/etc")).unwrap();
    std::fs::set_permissions(
        root.join("big/locked"),
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    let res = delete(&h, &c, &["big"], None).await;
    let id = ids(&res).first().cloned();
    let entry = root
        .join(".cairn-trash")
        .join(id.clone().unwrap_or_default())
        .join("big");
    // Let the temporary folder be removed whatever happens.
    let _ = std::fs::set_permissions(entry.join("locked"), std::fs::Permissions::from_mode(0o755));
    let _ = std::fs::set_permissions(
        root.join("big/locked"),
        std::fs::Permissions::from_mode(0o755),
    );
    assert_eq!(res.json["deleted"], 1, "{}", res.json);
    assert_eq!(trash_list(&h, &c).await[0]["size"], 5);
}

#[tokio::test]
async fn trash_round_trip_raw_name() {
    use std::os::unix::ffi::OsStrExt;
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    std::fs::create_dir_all(root.join("docs")).unwrap();
    let raw: &[u8] = b"caf\xe9.txt";
    std::fs::write(
        root.join("docs").join(std::ffi::OsStr::from_bytes(raw)),
        "c",
    )
    .unwrap();
    let seg = cairn::fs::path::RelPath::segment_for(raw);
    let res = delete(&h, &c, &[&format!("docs/{seg}")], None).await;
    assert_eq!(res.json["deleted"], 1, "{}", res.json);
    let e = &trash_list(&h, &c).await[0];
    assert_eq!(e["name"], "caf\u{FFFD}.txt");
    assert_eq!(e["rawName"], seg.as_str());
    let id = e["id"].as_str().unwrap().to_string();
    restore(&h, &c, &[&id]).await;
    assert_eq!(
        std::fs::read(root.join("docs").join(std::ffi::OsStr::from_bytes(raw))).unwrap(),
        b"c"
    );
}

#[tokio::test]
async fn undo_restores_by_entries() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "a.txt", "a");
    put(&root, "b.txt", "b");
    let res = delete(&h, &c, &["a.txt", "b.txt"], None).await;
    let got = ids(&res);
    let refs: Vec<&str> = got.iter().map(String::as_str).collect();
    let res = restore(&h, &c, &refs).await;
    assert_eq!(
        res.json["restored"].as_array().unwrap().len(),
        2,
        "{}",
        res.json
    );
    assert!(root.join("a.txt").exists() && root.join("b.txt").exists());
}

#[tokio::test]
async fn expired_entry_not_restorable() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "a.txt", "a");
    let id = ids(&delete(&h, &c, &["a.txt"], None).await)[0].clone();
    // Its date has come, but the hourly purge has not run yet.
    h.state
        .db
        .with(|db| db.execute("UPDATE trash SET expires_at = 1", []))
        .unwrap();
    assert!(trash_list(&h, &c).await.is_empty());
    let res = restore(&h, &c, &[&id]).await;
    assert_eq!(res.json["failed"][0]["error"], "not_found", "{}", res.json);
    assert!(!root.join("a.txt").exists());
}

#[test]
fn reconcile_waits_for_moves_in_flight() {
    let (h, _dir) = harness_with_root(&[]);
    let trash = h.state.trash.clone();
    let db = h.state.db.clone();
    let guard = h.state.trash.moving();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        cairn::trash::ops::reconcile(&trash, &db);
        cairn::trash::ops::purge(&trash, &db, cairn::now());
        tx.send(()).unwrap();
    });
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "reconcile ran while a move was in flight"
    );
    drop(guard);
    rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
}

#[tokio::test]
async fn folder_holding_mount_refused() {
    let (h, dir) = harness_with_root(&[]);
    let (c, root) = root_user(&h, &dir).await;
    put(&root, "media/photos/cat.jpg", "c");
    put(&root, "media/notes.txt", "n");
    cairn::fs::mount::fake_mount_point(&root.join("media/photos"));
    for permanent in [None, Some(true)] {
        let res = delete(&h, &c, &["media"], permanent).await;
        assert_eq!(
            res.json["failed"][0]["error"], "holds_mount",
            "{}",
            res.json
        );
        assert_eq!(
            res.json["failed"][0]["message"],
            "media has another disk mounted inside. Unmount it first.",
        );
    }
    // The mount point itself is refused too; what sits beside it is not.
    let res = delete(&h, &c, &["media/photos"], None).await;
    assert_eq!(
        res.json["failed"][0]["error"], "holds_mount",
        "{}",
        res.json
    );
    assert!(root.join("media/photos/cat.jpg").exists());
    let res = delete(&h, &c, &["media/notes.txt"], None).await;
    assert_eq!(res.json["deleted"], 1, "{}", res.json);
}
