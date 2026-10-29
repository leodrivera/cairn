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
    let folder = "Report #1 (50%) ✓ café";
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

const RAW_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; sandbox";

#[tokio::test]
async fn downloads_file() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    let name = "Report #1 (50%) ✓ café.txt";
    touch(&s.join(name), "hello world");
    let res = Req::get(&format!("/api/raw/{}", enc(name)))
        .cookie(&c)
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body, b"hello world");
    assert_eq!(res.header("content-security-policy"), RAW_CSP);
    assert_eq!(res.header("x-content-type-options"), "nosniff");
    assert_eq!(res.header("content-type"), "text/plain");
    assert_eq!(res.header("accept-ranges"), "bytes");
    assert_eq!(res.header("content-length"), "11");
    assert_eq!(
        res.header("content-disposition"),
        format!("inline; filename*=UTF-8''{}", enc(name))
    );
    assert!(
        res.header("etag").starts_with("\"11-"),
        "{}",
        res.header("etag")
    );
    assert!(!res.header("last-modified").is_empty());
}

#[tokio::test]
async fn download_param_is_attachment() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "a");
    let res = Req::get("/api/raw/a.txt?download=1")
        .cookie(&c)
        .send(&h.app)
        .await;
    assert!(
        res.header("content-disposition")
            .starts_with("attachment; "),
        "{}",
        res.header("content-disposition")
    );
}

#[tokio::test]
async fn range_206() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "0123456789");
    let res = Req::get("/api/raw/a.txt")
        .cookie(&c)
        .header("range", "bytes=2-4")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(res.body, b"234");
    assert_eq!(res.header("content-range"), "bytes 2-4/10");
    let tail = Req::get("/api/raw/a.txt")
        .cookie(&c)
        .header("range", "bytes=-3")
        .send(&h.app)
        .await;
    assert_eq!(tail.body, b"789");
}

#[tokio::test]
async fn range_416() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "0123456789");
    let res = Req::get("/api/raw/a.txt")
        .cookie(&c)
        .header("range", "bytes=10-")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(res.header("content-range"), "bytes */10");
}

#[tokio::test]
async fn etag_304() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "abc");
    let first = Req::get("/api/raw/a.txt").cookie(&c).send(&h.app).await;
    let etag = first.header("etag").to_string();
    let again = Req::get("/api/raw/a.txt")
        .cookie(&c)
        .header("if-none-match", &etag)
        .send(&h.app)
        .await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);
    assert!(again.body.is_empty());
}

#[tokio::test]
async fn folder_is_a_folder() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    std::fs::create_dir(s.join("d")).unwrap();
    let res = Req::get("/api/raw/d").cookie(&c).send(&h.app).await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::BAD_REQUEST, "is_a_folder")
    );
}

#[tokio::test]
async fn outside_link_403() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&dir.path().join("srv/bo/secret.txt"), "secret");
    symlink("../bo/secret.txt", s.join("leak")).unwrap();
    let res = Req::get("/api/raw/leak").cookie(&c).send(&h.app).await;
    assert_eq!((res.status, res.code()), (StatusCode::FORBIDDEN, "outside"));
    assert!(!String::from_utf8_lossy(&res.body).contains("secret\""));
}

fn temps(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".cairn-upload-"))
        .collect()
}

#[tokio::test]
async fn uploads_new_file() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    let res = Req::put(&format!("/api/upload/{}", enc("new ✓ café.txt")))
        .cookie(&c)
        .bytes(b"content")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(res.json["entry"]["name"], "new ✓ café.txt");
    assert_eq!(res.json["entry"]["size"], 7);
    assert_eq!(std::fs::read(s.join("new ✓ café.txt")).unwrap(), b"content");
    assert!(temps(&s).is_empty());
}

#[tokio::test]
async fn refuses_existing_without_replace() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "old");
    let res = Req::put("/api/upload/a.txt")
        .cookie(&c)
        .bytes(b"new")
        .send(&h.app)
        .await;
    assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "exists"));
    assert_eq!(std::fs::read_to_string(s.join("a.txt")).unwrap(), "old");
}

#[tokio::test]
async fn replaces_with_flag() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "old");
    let res = Req::put("/api/upload/a.txt?replace=1")
        .cookie(&c)
        .bytes(b"new")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(std::fs::read_to_string(s.join("a.txt")).unwrap(), "new");
}

#[tokio::test]
async fn parent_must_exist() {
    let (h, dir) = harness_with_root(&[]);
    let (c, _) = maria(&h, &dir, RW).await;
    let res = Req::put("/api/upload/nope/a.txt")
        .cookie(&c)
        .bytes(b"x")
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::NOT_FOUND, "not_found")
    );
}

#[tokio::test]
async fn upload_too_large() {
    let (h, dir) = harness_with_root(&[("CAIRN_UPLOAD_MAX", "10")]);
    let (c, s) = maria(&h, &dir, RW).await;
    let res = Req::put("/api/upload/big.bin")
        .cookie(&c)
        .bytes(b"01234567890")
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::PAYLOAD_TOO_LARGE, "too_large")
    );
    assert!(!s.join("big.bin").exists());
    assert!(temps(&s).is_empty());
    let ok = Req::put("/api/upload/ok.bin")
        .cookie(&c)
        .bytes(b"0123456789")
        .send(&h.app)
        .await;
    assert_eq!(ok.status, StatusCode::CREATED);
}

#[tokio::test]
async fn upload_needs_write() {
    let (h, dir) = harness_with_root(&[]);
    let ro = Perms {
        read: true,
        write: false,
        share: false,
        admin: false,
    };
    let (c, _) = maria(&h, &dir, ro).await;
    let res = Req::put("/api/upload/a.txt")
        .cookie(&c)
        .bytes(b"x")
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
}

#[tokio::test]
async fn temp_name_refused() {
    let (h, dir) = harness_with_root(&[]);
    let (c, _) = maria(&h, &dir, RW).await;
    let res = Req::put("/api/upload/.cairn-upload-x")
        .cookie(&c)
        .bytes(b"x")
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::BAD_REQUEST, "invalid_path")
    );
}

#[tokio::test]
async fn upload_bigger_than_json_limit() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    let big = vec![7u8; 3 << 20];
    let res = Req::put("/api/upload/big.bin")
        .cookie(&c)
        .bytes(&big)
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(std::fs::metadata(s.join("big.bin")).unwrap().len(), 3 << 20);
}

async fn delete_paths(h: &Harness, c: &str, paths: &[&str]) -> common::Res {
    Req::delete("/api/files")
        .cookie(c)
        .json(json!({ "paths": paths }))
        .send(&h.app)
        .await
}

#[tokio::test]
async fn mkdir_then_list() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    let res = Req::post(&format!("/api/folders/{}", enc("New folder ✓")))
        .cookie(&c)
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(res.json["entry"]["kind"], "folder");
    assert!(s.join("New folder ✓").is_dir());
}

#[tokio::test]
async fn mkdir_exists_409() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    std::fs::create_dir(s.join("d")).unwrap();
    let res = Req::post("/api/folders/d").cookie(&c).send(&h.app).await;
    assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "exists"));
    let temp = Req::post("/api/folders/.cairn-upload-x")
        .cookie(&c)
        .send(&h.app)
        .await;
    assert_eq!(temp.code(), "invalid_path");
}

#[tokio::test]
async fn delete_file_and_folder_tree() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "a");
    touch(&s.join("tree/x/y/z.txt"), "z");
    let res = delete_paths(&h, &c, &["a.txt", "tree"]).await;
    assert_eq!(res.json, json!({ "deleted": 2, "failed": [] }));
    assert!(!s.join("a.txt").exists() && !s.join("tree").exists());
}

#[tokio::test]
async fn delete_link_keeps_target() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("sub/g.txt"), "g");
    touch(&dir.path().join("srv/bo/secret.txt"), "s");
    symlink("sub/g.txt", s.join("in")).unwrap();
    symlink("../bo", s.join("up")).unwrap();
    symlink("sub", s.join("dirlink")).unwrap();
    let res = delete_paths(&h, &c, &["in", "up", "dirlink"]).await;
    assert_eq!(res.json["deleted"], 3, "{}", res.json);
    assert!(s.join("sub/g.txt").exists());
    assert!(dir.path().join("srv/bo/secret.txt").exists());
}

#[tokio::test]
async fn delete_reports_failures() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "a");
    let res = delete_paths(&h, &c, &["a.txt", "missing.txt"]).await;
    assert_eq!(res.json["deleted"], 1);
    assert_eq!(res.json["failed"][0]["path"], "missing.txt");
    assert_eq!(res.json["failed"][0]["error"], "not_found");
    let empty = delete_paths(&h, &c, &[]).await;
    assert_eq!(
        (empty.status, empty.code()),
        (StatusCode::BAD_REQUEST, "bad_request")
    );
}

#[tokio::test]
async fn cannot_delete_scope_root() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    let res = delete_paths(&h, &c, &[""]).await;
    assert_eq!(res.json["failed"][0]["error"], "scope_root");
    assert!(s.exists());
}

#[tokio::test]
async fn delete_unicode_name() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    let name = "Report #1 (50%) ✓ café.txt";
    touch(&s.join(name), "r");
    let res = delete_paths(&h, &c, &[name]).await;
    assert_eq!(res.json["deleted"], 1);
    assert!(!s.join(name).exists());
}

#[tokio::test]
async fn delete_needs_write() {
    let (h, dir) = harness_with_root(&[]);
    let ro = Perms {
        read: true,
        write: false,
        share: false,
        admin: false,
    };
    let (c, s) = maria(&h, &dir, ro).await;
    touch(&s.join("a.txt"), "a");
    let res = delete_paths(&h, &c, &["a.txt"]).await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
    assert!(s.join("a.txt").exists());
}

async fn ops(h: &Harness, c: &str, list: Value) -> common::Res {
    Req::post("/api/ops")
        .cookie(c)
        .json(json!({ "ops": list }))
        .send(&h.app)
        .await
}

#[tokio::test]
async fn rename_file() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "a");
    let res = ops(
        &h,
        &c,
        json!([{ "op": "rename", "from": "a.txt", "to": "b.txt" }]),
    )
    .await;
    assert_eq!(res.json, json!({ "done": 1, "failed": [] }));
    assert!(s.join("b.txt").exists() && !s.join("a.txt").exists());
}

#[tokio::test]
async fn rename_needs_same_parent() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "a");
    std::fs::create_dir(s.join("d")).unwrap();
    let res = ops(
        &h,
        &c,
        json!([{ "op": "rename", "from": "a.txt", "to": "d/a.txt" }]),
    )
    .await;
    assert_eq!(res.json["failed"][0]["error"], "invalid_path");
}

#[tokio::test]
async fn move_into_folder() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "a");
    std::fs::create_dir(s.join("d")).unwrap();
    let res = ops(
        &h,
        &c,
        json!([{ "op": "move", "from": "a.txt", "to": "d/a.txt" }]),
    )
    .await;
    assert_eq!(res.json["done"], 1, "{}", res.json);
    assert_eq!(std::fs::read_to_string(s.join("d/a.txt")).unwrap(), "a");
}

#[tokio::test]
async fn move_into_itself_refused() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    std::fs::create_dir_all(s.join("a/sub")).unwrap();
    let res = ops(
        &h,
        &c,
        json!([{ "op": "move", "from": "a", "to": "a/sub/a" }]),
    )
    .await;
    assert_eq!(res.json["failed"][0]["error"], "into_itself");
    let same = ops(&h, &c, json!([{ "op": "copy", "from": "a", "to": "a" }])).await;
    assert_eq!(same.json["failed"][0]["error"], "into_itself");
    assert!(s.join("a/sub").is_dir());
}

#[tokio::test]
async fn copy_tree() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("src/x.txt"), "x");
    touch(&s.join("src/deep/y.txt"), "y");
    let res = ops(
        &h,
        &c,
        json!([{ "op": "copy", "from": "src", "to": "dst" }]),
    )
    .await;
    assert_eq!(res.json["done"], 1, "{}", res.json);
    assert_eq!(std::fs::read_to_string(s.join("dst/x.txt")).unwrap(), "x");
    assert_eq!(
        std::fs::read_to_string(s.join("dst/deep/y.txt")).unwrap(),
        "y"
    );
    assert!(s.join("src/x.txt").exists());
}

#[tokio::test]
async fn copy_skips_outside_links() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&dir.path().join("srv/bo/secret.txt"), "s");
    touch(&s.join("src/mine.txt"), "m");
    symlink("../../bo", s.join("src/up")).unwrap();
    symlink("mine.txt", s.join("src/in")).unwrap();
    let res = ops(
        &h,
        &c,
        json!([{ "op": "copy", "from": "src", "to": "dst" }]),
    )
    .await;
    assert_eq!(res.json["done"], 1, "{}", res.json);
    assert!(
        std::fs::symlink_metadata(s.join("dst/in"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        std::fs::symlink_metadata(s.join("dst/up")).is_err(),
        "outside link copied"
    );
}

#[tokio::test]
async fn target_exists_409() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "a");
    touch(&s.join("b.txt"), "b");
    let res = ops(
        &h,
        &c,
        json!([{ "op": "rename", "from": "a.txt", "to": "b.txt" }]),
    )
    .await;
    assert_eq!(res.json["failed"][0]["error"], "exists");
    assert_eq!(std::fs::read_to_string(s.join("b.txt")).unwrap(), "b");
}

#[tokio::test]
async fn rename_unicode() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "a");
    let res = ops(
        &h,
        &c,
        json!([{ "op": "rename", "from": "a.txt", "to": "Report #1 (50%) ✓ café.txt" }]),
    )
    .await;
    assert_eq!(res.json["done"], 1);
    assert!(s.join("Report #1 (50%) ✓ café.txt").exists());
}

#[tokio::test]
async fn batch_reports_each() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "a");
    let res = ops(
        &h,
        &c,
        json!([
            { "op": "rename", "from": "a.txt", "to": "b.txt" },
            { "op": "move", "from": "missing", "to": "x" }
        ]),
    )
    .await;
    assert_eq!(res.json["done"], 1);
    assert_eq!(res.json["failed"][0]["from"], "missing");
    assert_eq!(res.json["failed"][0]["error"], "not_found");
}

#[tokio::test]
async fn scope_root_cannot_move() {
    let (h, dir) = harness_with_root(&[]);
    let (c, _) = maria(&h, &dir, RW).await;
    let res = ops(&h, &c, json!([{ "op": "move", "from": "", "to": "x" }])).await;
    assert_eq!(res.json["failed"][0]["error"], "scope_root");
}

/// A name written by an old system in Latin-1: "Report caf\xe9.txt" is not UTF-8.
fn latin1(dir: &Path) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let p = dir.join(std::ffi::OsStr::from_bytes(b"Report caf\xe9.txt"));
    std::fs::write(&p, "old").unwrap();
    p
}

#[tokio::test]
async fn non_utf8_name_is_listed_and_usable() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    let file = latin1(&s);
    let res = Req::get("/api/files").cookie(&c).send(&h.app).await;
    let e = &res.json["entries"][0];
    assert_eq!(e["name"], "Report caf\u{FFFD}.txt");
    let raw = e["rawName"].as_str().expect("rawName for a name that is not UTF-8").to_string();
    assert!(raw.starts_with('\0'));

    let dl = Req::get(&format!("/api/raw/{}", enc(&raw))).cookie(&c).send(&h.app).await;
    assert_eq!((dl.status, dl.body.clone()), (StatusCode::OK, b"old".to_vec()));

    let renamed = ops(&h, &c, json!([{ "op": "rename", "from": raw, "to": "Report café.txt" }])).await;
    assert_eq!(renamed.json["done"], 1, "{}", renamed.json);
    assert!(!file.exists());
    assert_eq!(std::fs::read_to_string(s.join("Report café.txt")).unwrap(), "old");
}

#[tokio::test]
async fn non_utf8_name_can_be_deleted() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    let file = latin1(&s);
    let list = Req::get("/api/files").cookie(&c).send(&h.app).await;
    let raw = list.json["entries"][0]["rawName"].as_str().unwrap().to_string();
    let res = delete_paths(&h, &c, &[raw.as_str()]).await;
    assert_eq!(res.json["deleted"], 1, "{}", res.json);
    assert!(!file.exists());
}

#[tokio::test]
async fn non_utf8_folder_contents_listed() {
    use std::os::unix::ffi::OsStrExt;
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    let folder = s.join(std::ffi::OsStr::from_bytes(b"Fotos f\xe9rias"));
    touch(&folder.join("a.jpg"), "a");
    let list = Req::get("/api/files").cookie(&c).send(&h.app).await;
    let raw = list.json["entries"][0]["rawName"].as_str().unwrap().to_string();
    let inside = Req::get(&format!("/api/files/{}", enc(&raw))).cookie(&c).send(&h.app).await;
    assert_eq!(inside.status, StatusCode::OK, "{}", inside.json);
    assert_eq!(names(&inside.json), ["a.jpg"]);
    assert_eq!(inside.json["path"], raw, "the path comes back in the form the client sent");
}
