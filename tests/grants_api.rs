//! Grants: sharing a file or folder with other Cairn users by e-mail, through the real router.

mod common;

use axum::http::StatusCode;
use cairn::users::Perms;
use common::{Harness, Req, harness_with_root};
use serde_json::{Value, json};

const PW: &str = "Abcdef1!";
const OWNER: Perms = Perms {
    read: true,
    write: true,
    share: true,
    admin: false,
};
const READ_SHARE: Perms = Perms {
    read: true,
    write: false,
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
    _dir: tempfile::TempDir,
    ana: String,
}

/// Ana (scope `/`, read, write, share) with `docs/notes.txt` and `photos/2026/`; Bob and Carol exist.
async fn world(owner: Perms) -> World {
    let (h, dir) = harness_with_root(&[]);
    let root = dir.path().join("srv");
    std::fs::create_dir_all(root.join("photos/2026")).unwrap();
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(root.join("docs/notes.txt"), "n").unwrap();
    std::fs::create_dir_all(root.join("bob")).unwrap();
    h.add_user_scoped("ana@example.com", PW, "/", owner);
    h.add_user_scoped("bob@example.com", PW, "/bob", PLAIN);
    h.add_user_scoped("carol@example.com", PW, "/bob", PLAIN);
    let ana = h.login("ana@example.com", PW).await;
    World { h, _dir: dir, ana }
}

async fn grant(w: &World, cookie: &str, body: Value) -> common::Res {
    Req::post("/api/grants")
        .cookie(cookie)
        .json(body)
        .send(&w.h.app)
        .await
}

fn rows(h: &Harness) -> i64 {
    h.state
        .db
        .with(|c| c.query_row("SELECT count(*) FROM grants", [], |r| r.get(0)))
        .unwrap()
}

fn statuses(res: &common::Res) -> Vec<(String, String)> {
    res.json["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["email"].as_str().unwrap().into(),
                r["status"].as_str().unwrap().into(),
            )
        })
        .collect()
}

#[tokio::test]
async fn create_for_several() {
    let w = world(OWNER).await;
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "photos", "emails": ["bob@example.com", "carol@example.com", "dan@example.com"], "permission": "write", "expiresDays": 7 }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(
        statuses(&res),
        vec![
            ("bob@example.com".into(), "added".into()),
            ("carol@example.com".into(), "added".into()),
            ("dan@example.com".into(), "no_account".into())
        ]
    );
    assert_eq!(
        res.json["results"][2]["message"],
        "No account for dan@example.com."
    );
    let g = &res.json["results"][0]["grant"];
    assert_eq!(g["recipient"]["email"], "bob@example.com");
    assert_eq!(g["permission"], "write");
    assert_eq!(g["effective"], "write");
    assert_eq!(g["kind"], "folder");
    assert_eq!(g["name"], "photos");
    assert_eq!(g["path"], "/photos");
    assert!(g["expiresAt"].as_i64().unwrap() > g["createdAt"].as_i64().unwrap());
    assert_eq!(rows(&w.h), 2);
}

#[tokio::test]
async fn create_self() {
    let w = world(OWNER).await;
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "photos", "emails": ["ana@example.com"], "permission": "read" }),
    )
    .await;
    assert_eq!(
        statuses(&res),
        vec![("ana@example.com".into(), "self".into())]
    );
    assert_eq!(res.json["results"][0]["message"], "That is you.");
    assert_eq!(rows(&w.h), 0);
}

#[tokio::test]
async fn grant_again_updates() {
    let w = world(OWNER).await;
    grant(&w, &w.ana, json!({ "path": "photos", "emails": ["bob@example.com"], "permission": "read", "expiresDays": 1 })).await;
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "photos", "emails": ["bob@example.com"], "permission": "write" }),
    )
    .await;
    assert_eq!(rows(&w.h), 1);
    assert_eq!(res.json["results"][0]["grant"]["permission"], "write");
    assert_eq!(res.json["results"][0]["grant"]["expiresAt"], Value::Null);
}

#[tokio::test]
async fn grant_needs_share() {
    let w = world(Perms {
        share: false,
        ..OWNER
    })
    .await;
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "photos", "emails": ["bob@example.com"], "permission": "read" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(rows(&w.h), 0);
}

#[tokio::test]
async fn write_grant_needs_owner_write() {
    let w = world(READ_SHARE).await;
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "photos", "emails": ["bob@example.com"], "permission": "write" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.json);
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "photos", "emails": ["bob@example.com"], "permission": "read" }),
    )
    .await;
    assert_eq!(
        statuses(&res),
        vec![("bob@example.com".into(), "added".into())]
    );
}

#[tokio::test]
async fn grant_path_must_exist_in_scope() {
    let w = world(OWNER).await;
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "nope", "emails": ["bob@example.com"], "permission": "read" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.json);
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "../x", "emails": ["bob@example.com"], "permission": "read" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(rows(&w.h), 0);
}

#[tokio::test]
async fn grant_emails_normalized() {
    let w = world(OWNER).await;
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "photos", "emails": [" Bob@Example.com", "bob@example.com", "  "], "permission": "read" }),
    )
    .await;
    assert_eq!(
        statuses(&res),
        vec![("bob@example.com".into(), "added".into())]
    );
    assert_eq!(rows(&w.h), 1);
    // Unknown, typed twice: counted once (19 more pass, then the limit).
    let res = grant(&w, &w.ana, json!({ "path": "photos", "emails": ["X@example.com", "x@example.com"], "permission": "read" })).await;
    assert_eq!(
        statuses(&res),
        vec![("x@example.com".into(), "no_account".into())]
    );
    for i in 0..19 {
        let res = grant(&w, &w.ana, json!({ "path": "photos", "emails": [format!("u{i}@example.com")], "permission": "read" })).await;
        assert_eq!(res.status, StatusCode::OK, "{i}");
    }
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "photos", "emails": ["last@example.com"], "permission": "read" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn unknown_emails_rate_limited() {
    let w = world(OWNER).await;
    let many: Vec<String> = (0..20).map(|i| format!("nobody{i}@example.com")).collect();
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "photos", "emails": many, "permission": "read" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "photos", "emails": ["bob@example.com", "nobody20@example.com"], "permission": "read" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{}", res.json);
    assert!(res.header("retry-after").parse::<i64>().unwrap() > 0);
    assert_eq!(rows(&w.h), 0, "nothing from a refused request is saved");
}

#[tokio::test]
async fn known_emails_not_counted() {
    let w = world(OWNER).await;
    for _ in 0..13 {
        let res = grant(
            &w,
            &w.ana,
            json!({ "path": "photos", "emails": ["bob@example.com", "carol@example.com"], "permission": "read" }),
        )
        .await;
        assert_eq!(res.status, StatusCode::OK);
    }
}

#[tokio::test]
async fn signed_out_cannot_ask() {
    let w = world(OWNER).await;
    let res = Req::post("/api/grants")
        .json(json!({ "path": "photos", "emails": ["bob@example.com"], "permission": "read" }))
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.code(), "signed_out");
    assert!(res.json.get("results").is_none());
}

#[tokio::test]
async fn max_twenty_per_request() {
    let w = world(OWNER).await;
    let many: Vec<String> = (0..21).map(|i| format!("p{i}@example.com")).collect();
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "photos", "emails": many, "permission": "read" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
}

async fn list(w: &World, cookie: &str, url: &str) -> Vec<Value> {
    let res = Req::get(url).cookie(cookie).send(&w.h.app).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    res.json["grants"].as_array().unwrap().clone()
}

#[tokio::test]
async fn list_given_and_received() {
    let w = world(OWNER).await;
    grant(
        &w,
        &w.ana,
        json!({ "path": "docs/notes.txt", "emails": ["bob@example.com"], "permission": "read" }),
    )
    .await;
    let given = list(&w, &w.ana, "/api/grants").await;
    assert_eq!(given.len(), 1);
    let bob = w.h.login("bob@example.com", PW).await;
    assert!(list(&w, &bob, "/api/grants").await.is_empty());
    let got = list(&w, &bob, "/api/grants/received").await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["name"], "notes.txt");
    assert_eq!(got[0]["kind"], "file");
    assert_eq!(got[0]["owner"]["name"], "Someone");
    assert!(
        got[0].get("path").is_none(),
        "the recipient never learns the owner's path"
    );
    w.h.add_user("admin@example.com", Some(PW), true);
    let admin = w.h.login("admin@example.com", PW).await;
    assert_eq!(list(&w, &admin, "/api/grants").await.len(), 1);
}

#[tokio::test]
async fn received_hides_dead() {
    let w = world(OWNER).await;
    let res = grant(&w, &w.ana, json!({ "path": "photos", "emails": ["bob@example.com"], "permission": "read", "expiresDays": 1 })).await;
    let id = res.json["results"][0]["grant"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    w.h.state
        .db
        .with(|c| c.execute("UPDATE grants SET expires_at = 1 WHERE id = ?1", [&id]))
        .unwrap();
    let bob = w.h.login("bob@example.com", PW).await;
    assert!(list(&w, &bob, "/api/grants/received").await.is_empty());
}

#[tokio::test]
async fn patch_and_delete_rules() {
    let w = world(OWNER).await;
    let res = grant(
        &w,
        &w.ana,
        json!({ "path": "photos", "emails": ["bob@example.com"], "permission": "read" }),
    )
    .await;
    let id = res.json["results"][0]["grant"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let url = format!("/api/grants/{id}");
    let bob = w.h.login("bob@example.com", PW).await;
    let carol = w.h.login("carol@example.com", PW).await;
    let res = Req::patch(&url)
        .cookie(&bob)
        .json(json!({ "permission": "write" }))
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = Req::delete(&url).cookie(&carol).send(&w.h.app).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = Req::patch(&url)
        .cookie(&w.ana)
        .json(json!({ "permission": "write", "expiresDays": 30 }))
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["grant"]["permission"], "write");
    // Owner without share: no change, but may still revoke.
    let ana = cairn::users::by_email(&w.h.state.db, "ana@example.com")
        .unwrap()
        .unwrap()
        .0;
    w.h.state
        .db
        .with(|c| c.execute("UPDATE users SET perm_share = 0 WHERE id = ?1", [ana.id]))
        .unwrap();
    let res = Req::patch(&url)
        .cookie(&w.ana)
        .json(json!({ "permission": "read" }))
        .send(&w.h.app)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    // The recipient leaves.
    let res = Req::delete(&url).cookie(&bob).send(&w.h.app).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert_eq!(rows(&w.h), 0);
}

#[tokio::test]
async fn deleting_user_deletes_grants() {
    let w = world(OWNER).await;
    grant(&w, &w.ana, json!({ "path": "photos", "emails": ["bob@example.com", "carol@example.com"], "permission": "read" })).await;
    assert_eq!(rows(&w.h), 2);
    let bob = cairn::users::by_email(&w.h.state.db, "bob@example.com")
        .unwrap()
        .unwrap()
        .0;
    cairn::users::delete(&w.h.state.db, bob.id).unwrap();
    assert_eq!(rows(&w.h), 1);
    let ana = cairn::users::by_email(&w.h.state.db, "ana@example.com")
        .unwrap()
        .unwrap()
        .0;
    cairn::users::delete(&w.h.state.db, ana.id).unwrap();
    assert_eq!(rows(&w.h), 0);
}
