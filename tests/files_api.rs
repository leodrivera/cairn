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

/// A permanent delete: these tests are about removing files; the trash has its own (tests/trash_api.rs).
async fn delete_paths(h: &Harness, c: &str, paths: &[&str]) -> common::Res {
    Req::delete("/api/files")
        .cookie(c)
        .json(json!({ "paths": paths, "permanent": true }))
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
    assert_eq!(
        res.json,
        json!({ "deleted": 2, "failed": [], "entries": [] })
    );
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
    // The copy happens, and the left-out link is reported, not silently dropped.
    assert_eq!(res.json["failed"][0]["error"], "skipped", "{}", res.json);
    assert_eq!(
        res.json["failed"][0]["message"],
        "1 item was left out (links that leave your folder, or special files). The rest was copied."
    );
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
    let raw = e["rawName"]
        .as_str()
        .expect("rawName for a name that is not UTF-8")
        .to_string();
    assert!(raw.starts_with('\0'));

    let dl = Req::get(&format!("/api/raw/{}", enc(&raw)))
        .cookie(&c)
        .send(&h.app)
        .await;
    assert_eq!(
        (dl.status, dl.body.clone()),
        (StatusCode::OK, b"old".to_vec())
    );

    let renamed = ops(
        &h,
        &c,
        json!([{ "op": "rename", "from": raw, "to": "Report café.txt" }]),
    )
    .await;
    assert_eq!(renamed.json["done"], 1, "{}", renamed.json);
    assert!(!file.exists());
    assert_eq!(
        std::fs::read_to_string(s.join("Report café.txt")).unwrap(),
        "old"
    );
}

#[tokio::test]
async fn non_utf8_name_can_be_deleted() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    let file = latin1(&s);
    let list = Req::get("/api/files").cookie(&c).send(&h.app).await;
    let raw = list.json["entries"][0]["rawName"]
        .as_str()
        .unwrap()
        .to_string();
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
    let raw = list.json["entries"][0]["rawName"]
        .as_str()
        .unwrap()
        .to_string();
    let inside = Req::get(&format!("/api/files/{}", enc(&raw)))
        .cookie(&c)
        .send(&h.app)
        .await;
    assert_eq!(inside.status, StatusCode::OK, "{}", inside.json);
    assert_eq!(names(&inside.json), ["a.jpg"]);
    assert_eq!(
        inside.json["path"], raw,
        "the path comes back in the form the client sent"
    );
}

async fn etag_of(h: &Harness, c: &str, path: &str) -> String {
    Req::get(&format!("/api/raw/{path}"))
        .cookie(c)
        .send(&h.app)
        .await
        .header("etag")
        .to_string()
}

#[tokio::test]
async fn save_with_matching_etag() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("notes.txt"), "old");
    let tag = etag_of(&h, &c, "notes.txt").await;
    let res = Req::put("/api/text/notes.txt")
        .cookie(&c)
        .header("if-match", &tag)
        .bytes("new ✓".as_bytes())
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(
        std::fs::read_to_string(s.join("notes.txt")).unwrap(),
        "new ✓"
    );
    assert_eq!(res.json["etag"], res.header("etag"));
    assert_ne!(res.header("etag"), tag);
}

#[tokio::test]
async fn stale_etag_412() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("notes.txt"), "old");
    let res = Req::put("/api/text/notes.txt")
        .cookie(&c)
        .header("if-match", "\"3-0-0\"")
        .bytes(b"new")
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::PRECONDITION_FAILED, "changed")
    );
    assert_eq!(std::fs::read_to_string(s.join("notes.txt")).unwrap(), "old");
}

#[tokio::test]
async fn missing_if_match_428() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("notes.txt"), "old");
    let res = Req::put("/api/text/notes.txt")
        .cookie(&c)
        .bytes(b"new")
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::PRECONDITION_REQUIRED, "if_match_required")
    );
}

#[tokio::test]
async fn new_file_with_if_none_match() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    let res = Req::put("/api/text/new.md")
        .cookie(&c)
        .header("if-none-match", "*")
        .bytes(b"# Title")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(
        std::fs::read_to_string(s.join("new.md")).unwrap(),
        "# Title"
    );
    let again = Req::put("/api/text/new.md")
        .cookie(&c)
        .header("if-none-match", "*")
        .bytes(b"x")
        .send(&h.app)
        .await;
    assert_eq!(again.code(), "exists");
}

#[tokio::test]
async fn text_too_large_413() {
    let (h, dir) = harness_with_root(&[("CAIRN_EDIT_MAX", "8")]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("n.txt"), "old");
    let tag = etag_of(&h, &c, "n.txt").await;
    let res = Req::put("/api/text/n.txt")
        .cookie(&c)
        .header("if-match", &tag)
        .bytes(b"123456789")
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::PAYLOAD_TOO_LARGE, "too_large")
    );
    assert_eq!(std::fs::read_to_string(s.join("n.txt")).unwrap(), "old");
}

#[tokio::test]
async fn binary_415() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("n.txt"), "old");
    let tag = etag_of(&h, &c, "n.txt").await;
    let res = Req::put("/api/text/n.txt")
        .cookie(&c)
        .header("if-match", &tag)
        .bytes(&[0xff, 0xfe, 0x00])
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::UNSUPPORTED_MEDIA_TYPE, "not_text")
    );
}

#[tokio::test]
async fn etag_matches_raw() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("n.txt"), "old");
    let tag = etag_of(&h, &c, "n.txt").await;
    let res = Req::put("/api/text/n.txt")
        .cookie(&c)
        .header("if-match", &tag)
        .bytes(b"newer")
        .send(&h.app)
        .await;
    assert_eq!(res.header("etag"), etag_of(&h, &c, "n.txt").await);
}

#[tokio::test]
async fn save_needs_write() {
    let (h, dir) = harness_with_root(&[]);
    let ro = Perms {
        read: true,
        write: false,
        share: false,
        admin: false,
    };
    let (c, s) = maria(&h, &dir, ro).await;
    touch(&s.join("n.txt"), "old");
    let res = Req::put("/api/text/n.txt")
        .cookie(&c)
        .header("if-none-match", "*")
        .bytes(b"x")
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
}

fn unzip(body: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(body.to_vec())).expect("a valid ZIP");
    let mut out = std::collections::BTreeMap::new();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).unwrap();
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut data).unwrap();
        out.insert(f.name().unwrap().to_string(), data);
    }
    out
}

#[tokio::test]
async fn zip_contains_tree() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("album/a.txt"), "a");
    touch(&s.join("album/deep/b.txt"), "bb");
    std::fs::create_dir(s.join("album/empty")).unwrap();
    let res = Req::get("/api/zip/album").cookie(&c).send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("content-type"), "application/zip");
    assert_eq!(
        res.header("content-disposition"),
        "attachment; filename*=UTF-8''album.zip"
    );
    assert_eq!(res.header("content-security-policy"), RAW_CSP);
    let files = unzip(&res.body);
    assert_eq!(files.get("a.txt").map(Vec::as_slice), Some(&b"a"[..]));
    assert_eq!(files.get("deep/b.txt").map(Vec::as_slice), Some(&b"bb"[..]));
    assert!(files.contains_key("empty/"), "{:?}", files.keys());
}

#[tokio::test]
async fn zip_of_root_is_files_zip() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("x.txt"), "x");
    let res = Req::get("/api/zip").cookie(&c).send(&h.app).await;
    assert_eq!(
        res.header("content-disposition"),
        "attachment; filename*=UTF-8''files.zip"
    );
    assert!(unzip(&res.body).contains_key("x.txt"));
}

#[tokio::test]
async fn zip_skips_outside_links() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&dir.path().join("srv/bo/secret.txt"), "s");
    touch(&s.join("album/a.txt"), "a");
    symlink("../../bo", s.join("album/up")).unwrap();
    symlink("../../bo/secret.txt", s.join("album/leak.txt")).unwrap();
    symlink("a.txt", s.join("album/in.txt")).unwrap();
    let files = unzip(
        &Req::get("/api/zip/album")
            .cookie(&c)
            .send(&h.app)
            .await
            .body,
    );
    assert!(
        !files.keys().any(|k| k.starts_with("up") || k == "leak.txt"),
        "{:?}",
        files.keys()
    );
    assert_eq!(files.get("in.txt").map(Vec::as_slice), Some(&b"a"[..]));
}

#[tokio::test]
async fn zip_of_file_is_not_a_folder() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "a");
    let res = Req::get("/api/zip/a.txt").cookie(&c).send(&h.app).await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::BAD_REQUEST, "not_a_folder")
    );
}

#[tokio::test]
async fn zip_needs_read() {
    let (h, dir) = harness_with_root(&[]);
    let (c, _) = maria(&h, &dir, NONE).await;
    let res = Req::get("/api/zip").cookie(&c).send(&h.app).await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
}

#[tokio::test]
async fn fallback_mode_lists_downloads_and_uploads() {
    let (h, dir) = harness_with_root(&[]);
    let h = h.with_fallback();
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("sub/a.txt"), "a");
    let list = Req::get("/api/files/sub").cookie(&c).send(&h.app).await;
    assert_eq!(names(&list.json), ["a.txt"]);
    let dl = Req::get("/api/raw/sub/a.txt").cookie(&c).send(&h.app).await;
    assert_eq!(dl.body, b"a");
    let up = Req::put("/api/upload/sub/b.txt")
        .cookie(&c)
        .bytes(b"b")
        .send(&h.app)
        .await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", up.json);
    assert_eq!(std::fs::read_to_string(s.join("sub/b.txt")).unwrap(), "b");
}

#[tokio::test]
async fn fallback_mode_refuses_links() {
    let (h, dir) = harness_with_root(&[]);
    let h = h.with_fallback();
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a.txt"), "a");
    touch(&dir.path().join("srv/bo/secret.txt"), "s");
    symlink("a.txt", s.join("in")).unwrap();
    symlink("../bo/secret.txt", s.join("leak")).unwrap();
    for link in ["in", "leak"] {
        let res = Req::get(&format!("/api/raw/{link}"))
            .cookie(&c)
            .send(&h.app)
            .await;
        assert_eq!(
            (res.status, res.code()),
            (StatusCode::FORBIDDEN, "outside"),
            "{link}"
        );
    }
}

#[tokio::test]
async fn copy_into_itself_through_a_link() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("a/x.txt"), "x");
    symlink("a", s.join("L")).unwrap();
    let res = ops(
        &h,
        &c,
        json!([{ "op": "copy", "from": "a", "to": "L/sub" }]),
    )
    .await;
    assert_eq!(
        res.json["failed"][0]["error"], "into_itself",
        "{}",
        res.json
    );
    let moved = ops(
        &h,
        &c,
        json!([{ "op": "move", "from": "a", "to": "L/sub" }]),
    )
    .await;
    assert_eq!(
        moved.json["failed"][0]["error"], "into_itself",
        "{}",
        moved.json
    );
    assert!(s.join("a/x.txt").exists() && !s.join("a/sub").exists());
}

#[tokio::test]
async fn copy_keeps_non_utf8_names() {
    use std::os::unix::ffi::OsStrExt;
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(
        &s.join("src")
            .join(std::ffi::OsStr::from_bytes(b"Report caf\xe9.txt")),
        "old",
    );
    let res = ops(
        &h,
        &c,
        json!([{ "op": "copy", "from": "src", "to": "dst" }]),
    )
    .await;
    assert_eq!(res.json, json!({ "done": 1, "failed": [] }));
    assert!(
        s.join("dst")
            .join(std::ffi::OsStr::from_bytes(b"Report caf\xe9.txt"))
            .exists()
    );
}

/// A chain of folders `d/d/d/...`, deeper than Cairn walks.
fn deep_tree(base: &Path, levels: usize) {
    let mut p = base.to_path_buf();
    for _ in 0..levels {
        p.push("d");
    }
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(p.join("f.txt"), "f").unwrap();
}

#[tokio::test]
async fn deep_tree_delete_and_copy_stop_cleanly() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    deep_tree(&s.join("deep"), 300);
    let copy = ops(
        &h,
        &c,
        json!([{ "op": "copy", "from": "deep", "to": "copy" }]),
    )
    .await;
    assert_eq!(copy.json["failed"][0]["error"], "too_deep", "{}", copy.json);
    let del = delete_paths(&h, &c, &["deep"]).await;
    assert_eq!(del.json["failed"][0]["error"], "too_deep", "{}", del.json);
    assert!(s.join("deep").exists());
}

#[tokio::test]
async fn permission_denied_on_disk_is_403_denied() {
    use std::os::unix::fs::PermissionsExt;
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    std::fs::create_dir(s.join("locked")).unwrap();
    std::fs::set_permissions(s.join("locked"), std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::write(s.join("locked/probe"), b"").is_ok() {
        return; // running as root: permissions do not apply
    }
    let res = Req::put("/api/upload/locked/a.txt")
        .cookie(&c)
        .bytes(b"x")
        .send(&h.app)
        .await;
    std::fs::set_permissions(s.join("locked"), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::FORBIDDEN, "denied"),
        "{}",
        res.json
    );
}

#[tokio::test]
async fn replacing_keeps_the_original_mode() {
    use std::os::unix::fs::PermissionsExt;
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("run.sh"), "#!/bin/sh\necho old\n");
    std::fs::set_permissions(s.join("run.sh"), std::fs::Permissions::from_mode(0o750)).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o7777;

    let up = Req::put("/api/upload/run.sh?replace=1")
        .cookie(&c)
        .bytes(b"#!/bin/sh\necho new\n")
        .send(&h.app)
        .await;
    assert_eq!(up.status, StatusCode::OK, "{}", up.json);
    assert_eq!(mode(&s.join("run.sh")), 0o750, "upload with replace");

    let tag = etag_of(&h, &c, "run.sh").await;
    let save = Req::put("/api/text/run.sh")
        .cookie(&c)
        .header("if-match", &tag)
        .bytes(b"#!/bin/sh\necho newer\n")
        .send(&h.app)
        .await;
    assert_eq!(save.status, StatusCode::OK, "{}", save.json);
    assert_eq!(mode(&s.join("run.sh")), 0o750, "text save");
}

#[tokio::test]
async fn space_for_admin_only() {
    let (h, dir) = harness_with_root(&[]);
    let (c, _) = maria(&h, &dir, RW).await;
    let res = Req::get("/api/files").cookie(&c).send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.json.get("space").is_none(), "{}", res.json);
    let admin = Perms {
        read: true,
        write: true,
        share: true,
        admin: true,
    };
    h.add_user_scoped("admin@example.com", PW, "/", admin);
    let a = h.login("admin@example.com", PW).await;
    let res = Req::get("/api/files").cookie(&a).send(&h.app).await;
    assert!(
        res.json["space"]["total"].as_u64().unwrap() > 0,
        "{}",
        res.json
    );
    assert!(res.json["space"]["used"].as_u64().is_some());
}

/// `album/a.txt`, `album/b.txt`, `album/sub/c.txt`.
async fn album(h: &Harness, dir: &tempfile::TempDir) -> String {
    let (c, s) = maria(h, dir, RW).await;
    touch(&s.join("album/a.txt"), "a");
    touch(&s.join("album/b.txt"), "b");
    touch(&s.join("album/sub/c.txt"), "c");
    c
}

/// A ZIP of chosen items as a browser form sends it: no X-Cairn, an Origin of the same host.
fn zip_form(path: &str, form: &str, cookie: &str) -> Req {
    Req::post(path)
        .no_csrf()
        .cookie(cookie)
        .header("host", "files.example.com")
        .header("origin", "https://files.example.com")
        .header("content-type", "application/x-www-form-urlencoded")
        .bytes(form.as_bytes())
}

#[tokio::test]
async fn zip_chosen_items() {
    let (h, dir) = harness_with_root(&[]);
    let c = album(&h, &dir).await;
    let res = zip_form("/api/zip/album", "item=a.txt&item=sub", &c)
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(
        res.header("content-disposition"),
        "attachment; filename*=UTF-8''album.zip"
    );
    let files = unzip(&res.body);
    assert_eq!(files.get("a.txt").map(Vec::as_slice), Some(&b"a"[..]));
    assert_eq!(files.get("sub/c.txt").map(Vec::as_slice), Some(&b"c"[..]));
    assert!(!files.contains_key("b.txt"), "{:?}", files.keys());
}

#[tokio::test]
async fn zip_chosen_items_at_root() {
    let (h, dir) = harness_with_root(&[]);
    let c = album(&h, &dir).await;
    let res = zip_form("/api/zip", "item=album", &c).send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(
        res.header("content-disposition"),
        "attachment; filename*=UTF-8''files.zip"
    );
    assert!(unzip(&res.body).contains_key("album/a.txt"));
}

#[tokio::test]
async fn zip_items_unicode() {
    let (h, dir) = harness_with_root(&[]);
    let (c, s) = maria(&h, &dir, RW).await;
    touch(&s.join("album/Report #1 (50%) ✓ café.txt"), "r");
    // Forms send spaces as `+`.
    let form = format!(
        "item={}",
        enc("Report #1 (50%) ✓ café.txt").replace("%20", "+")
    );
    let res = zip_form("/api/zip/album", &form, &c).send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(unzip(&res.body).contains_key("Report #1 (50%) ✓ café.txt"));
}

#[tokio::test]
async fn zip_items_refuses_slash_and_duplicates() {
    let (h, dir) = harness_with_root(&[]);
    let c = album(&h, &dir).await;
    for q in [
        "item=sub%2Fc.txt",
        "item=a.txt&item=a.txt",
        "item=",
        "item=..",
        "",
        "other=a.txt",
    ] {
        let res = zip_form("/api/zip/album", q, &c).send(&h.app).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{q}");
        assert_eq!(res.code(), "bad_request", "{q}");
    }
}

#[tokio::test]
async fn zip_items_missing_404() {
    let (h, dir) = harness_with_root(&[]);
    let c = album(&h, &dir).await;
    let res = zip_form("/api/zip/album", "item=a.txt&item=nope.txt", &c)
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.code(), "not_found");
}

#[tokio::test]
async fn zip_items_cap() {
    let (h, dir) = harness_with_root(&[]);
    let c = album(&h, &dir).await;
    let q: Vec<String> = (0..1001).map(|i| format!("item=f{i}")).collect();
    let res = zip_form("/api/zip/album", &q.join("&"), &c)
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn zip_form_from_another_site_refused() {
    let (h, dir) = harness_with_root(&[]);
    let c = album(&h, &dir).await;
    for origin in [Some("https://evil.example.net"), Some("null"), None] {
        let mut req = Req::post("/api/zip/album")
            .no_csrf()
            .cookie(&c)
            .header("host", "files.example.com")
            .header("content-type", "application/x-www-form-urlencoded")
            .bytes(b"item=a.txt");
        if let Some(o) = origin {
            req = req.header("origin", o);
        }
        let res = req.send(&h.app).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{origin:?}");
        assert_eq!(res.code(), "csrf", "{origin:?}");
    }
}

#[tokio::test]
async fn other_posts_still_need_x_cairn() {
    let (h, dir) = harness_with_root(&[]);
    let c = album(&h, &dir).await;
    let res = Req::post("/api/folders/new")
        .no_csrf()
        .cookie(&c)
        .header("host", "files.example.com")
        .header("origin", "https://files.example.com")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.code(), "csrf");
}

#[tokio::test]
async fn zip_form_behind_a_proxy_that_rewrites_host() {
    let (h, dir) = harness_with_root(&[("CAIRN_PUBLIC_URL", "https://files.example.com")]);
    let c = album(&h, &dir).await;
    // nginx by default sends the upstream name as Host.
    let res = Req::post("/api/zip/album")
        .no_csrf()
        .cookie(&c)
        .header("host", "cairn:8080")
        .header("origin", "https://files.example.com")
        .header("content-type", "application/x-www-form-urlencoded")
        .bytes(b"item=a.txt")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
}

#[tokio::test]
async fn zip_form_same_origin_fetch_metadata() {
    let (h, dir) = harness_with_root(&[]);
    let c = album(&h, &dir).await;
    let res = Req::post("/api/zip/album")
        .no_csrf()
        .cookie(&c)
        .header("host", "cairn:8080")
        .header("origin", "https://files.example.com")
        .header("sec-fetch-site", "same-origin")
        .header("content-type", "application/x-www-form-urlencoded")
        .bytes(b"item=a.txt")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
}

#[tokio::test]
async fn zip_form_cross_site_fetch_metadata_refused() {
    let (h, dir) = harness_with_root(&[]);
    let c = album(&h, &dir).await;
    // A matching Host does not help when the browser says the form came from another site.
    let res = Req::post("/api/zip/album")
        .no_csrf()
        .cookie(&c)
        .header("host", "files.example.com")
        .header("origin", "https://files.example.com")
        .header("sec-fetch-site", "cross-site")
        .header("content-type", "application/x-www-form-urlencoded")
        .bytes(b"item=a.txt")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.code(), "csrf");
}

#[tokio::test]
async fn pipe_answers_at_once() {
    let (h, dir) = harness_with_root(&[]);
    let pipe = dir.path().join("srv/pipe");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &pipe,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o644),
        0,
    )
    .unwrap();
    h.add_user("ana@example.com", Some(PW), false);
    let c = h.login("ana@example.com", PW).await;
    let read = Req::get("/api/raw/pipe").cookie(&c).send(&h.app);
    let res = tokio::time::timeout(std::time::Duration::from_secs(5), read).await;
    // Opened by a writer, a stuck reader lets go, so a failure here cannot hang the test's runtime.
    let _writer = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&pipe)
        .unwrap();
    assert_eq!(res.expect("no answer").status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn socket_answers_404() {
    let (h, dir) = harness_with_root(&[]);
    let _socket = std::os::unix::net::UnixListener::bind(dir.path().join("srv/sock")).unwrap();
    h.add_user("ana@example.com", Some(PW), false);
    let c = h.login("ana@example.com", PW).await;
    let res = Req::get("/api/raw/sock").cookie(&c).send(&h.app).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}
