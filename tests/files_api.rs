//! File routes, through the real router, on a real folder.

mod common;

use std::os::unix::fs::symlink;
use std::path::Path;

use axum::http::StatusCode;
use cairn::users::Perms;
use common::{Harness, Req, enc, harness_with_root};
use serde_json::{Value, json};

const PW: &str = "Abcdef1!";
const RW: Perms = Perms {
    read: true,
    write: true,
    share: false,
    admin: false,
};
const NONE: Perms = Perms {
    read: false,
    write: false,
    share: false,
    admin: false,
};

/// A user with scope `/maria`, signed in. Returns the cookie and the scope folder on disk.
async fn maria(h: &Harness, dir: &tempfile::TempDir, perms: Perms) -> (String, std::path::PathBuf) {
    let scope = dir.path().join("srv/maria");
    std::fs::create_dir_all(&scope).unwrap();
    h.add_user_scoped("maria@example.com", PW, "/maria", perms);
    (h.login("maria@example.com", PW).await, scope)
}

fn names(v: &Value) -> Vec<String> {
    v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect()
}

fn touch(p: &Path, content: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

#[tokio::test]
async fn lists_root_folders_first() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("b.txt"), "bb");
    touch(&s.join("a.txt"), "a");
    std::fs::create_dir(s.join("zeta")).unwrap();
    let res = Req::get("/api/files").cookie(&c).send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["path"], "");
    assert_eq!(names(&res.json), ["zeta", "a.txt", "b.txt"]);
    let b = &res.json["entries"][2];
    assert_eq!(
        (b["kind"].as_str(), b["size"].as_u64(), b["mime"].as_str()),
        (Some("file"), Some(2), Some("text/plain"))
    );
    assert_eq!(res.json["entries"][0]["kind"], "folder");
    assert!(b.get("outside").is_none());
}

#[tokio::test]
async fn lists_subfolder_with_unicode_name() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    let folder = "Relatório #1 (50%) ✓";
    touch(&s.join(folder).join("x.txt"), "x");
    let res = Req::get(&format!("/api/files/{}", enc(folder)))
        .cookie(&c)
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["path"], folder);
    assert_eq!(names(&res.json), ["x.txt"]);
}

#[tokio::test]
async fn hides_upload_temps() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join(".cairn-upload-0123456789abcdef"), "partial");
    touch(&s.join(".hidden"), "shown");
    let res = Req::get("/api/files").cookie(&c).send(&h.app).await;
    assert_eq!(names(&res.json), [".hidden"]);
}

#[tokio::test]
async fn outside_link_marked() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&dir.path().join("srv/bo/secret.txt"), "s");
    touch(&s.join("mine.txt"), "m");
    symlink("../bo", s.join("to-bo")).unwrap();
    symlink("mine.txt", s.join("to-mine")).unwrap();
    let res = Req::get("/api/files").cookie(&c).send(&h.app).await;
    let entries = res.json["entries"].as_array().unwrap();
    let find = |n: &str| entries.iter().find(|e| e["name"] == n).unwrap().clone();
    assert_eq!(find("to-bo")["outside"], true);
    assert_eq!(find("to-bo")["kind"], "link");
    assert_eq!(find("to-mine")["kind"], "file");
    assert_eq!(find("to-mine")["size"], 1);
    let through = Req::get("/api/files/to-bo").cookie(&c).send(&h.app).await;
    assert_eq!(
        (through.status, through.code()),
        (StatusCode::FORBIDDEN, "outside")
    );
}

#[tokio::test]
async fn file_is_not_a_folder() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "a");
    let res = Req::get("/api/files/a.txt").cookie(&c).send(&h.app).await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::BAD_REQUEST, "not_a_folder")
    );
}

#[tokio::test]
async fn missing_is_404() {
    let (h, dir) = harness_with_root(&[]);
    let (c, _) = maria(&h, &dir, RW).await;
    let res = Req::get("/api/files/nope").cookie(&c).send(&h.app).await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::NOT_FOUND, "not_found")
    );
    let bad = Req::get("/api/files/a/../b").cookie(&c).send(&h.app).await;
    assert_ne!(bad.status, StatusCode::OK);
}

#[tokio::test]
async fn needs_read() {
    let (h, dir) = harness_with_root(&[]);
    let (c, _) = maria(&h, &dir, NONE).await;
    let res = Req::get("/api/files").cookie(&c).send(&h.app).await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
}

#[tokio::test]
async fn scope_applies() {
    let (h, dir) = harness_with_root(&[]);
    touch(&dir.path().join("srv/top.txt"), "t");
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("inside.txt"), "i");
    let res = Req::get("/api/files").cookie(&c).send(&h.app).await;
    assert_eq!(names(&res.json), ["inside.txt"]);
}

#[tokio::test]
async fn signed_out_403() {
    let (h, _dir) = harness_with_root(&[]);
    let res = Req::get("/api/files").send(&h.app).await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::FORBIDDEN, "signed_out")
    );
    let _ = json!(null);
}
