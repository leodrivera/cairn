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
