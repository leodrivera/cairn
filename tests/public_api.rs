//! The public share page and its file routes under `/s/{id}`.

mod common;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use axum::http::StatusCode;
use cairn::users::{self, Perms, UserPatch};
use common::{Harness, Req, Res, enc, harness_with_root};
use serde_json::{Value, json};

const PW: &str = "Abcdef1!";
const RAW_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; sandbox";
const APP_CSP: &str = "default-src 'self'; img-src 'self' blob: data:; style-src 'self'; script-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
const GONE: &str = "This link no longer works. Ask the person who sent it for a new one.";

struct Fx {
    h: Harness,
    dir: tempfile::TempDir,
    admin: String,
}

/// `srv/photos/2026/trail-ridge.jpg`, `srv/photos/notes.txt`, `srv/photos/page.html`, `srv/photos/sub/deep.txt`.
async fn fx() -> Fx {
    let (h, dir) = harness_with_root(&[]);
    let srv = dir.path().join("srv");
    std::fs::create_dir_all(srv.join("photos/2026")).unwrap();
    std::fs::create_dir_all(srv.join("photos/sub")).unwrap();
    std::fs::write(srv.join("photos/2026/trail-ridge.jpg"), "jpeg bytes").unwrap();
    std::fs::write(srv.join("photos/notes.txt"), "some notes").unwrap();
    std::fs::write(srv.join("photos/page.html"), "<script>alert(1)</script>").unwrap();
    std::fs::write(srv.join("photos/sub/deep.txt"), "deep").unwrap();
    std::fs::write(srv.join("secret.txt"), "secret").unwrap();
    h.add_user("admin@example.com", Some(PW), true);
    let admin = h.login("admin@example.com", PW).await;
    Fx { h, dir, admin }
}

impl Fx {
    async fn share(&self, body: Value) -> String {
        let res = Req::post("/api/shares")
            .cookie(&self.admin)
            .json(body)
            .send(&self.h.app)
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
        res.json["share"]["id"].as_str().unwrap().to_string()
    }

    async fn file(&self) -> String {
        self.share(
            json!({ "path": "photos/2026/trail-ridge.jpg", "expiresDays": 7, "download": true }),
        )
        .await
    }

    async fn folder(&self, download: bool) -> String {
        self.share(json!({ "path": "photos", "expiresDays": 7, "download": download }))
            .await
    }

    /// Every answer here goes through this: none may be 401, and none needs a session.
    async fn get(&self, path: &str) -> Res {
        let res = Req::get(path).send(&self.h.app).await;
        assert_ne!(res.status, StatusCode::UNAUTHORIZED, "{path}");
        res
    }

    async fn hits(&self) -> Vec<i64> {
        let res = Req::get("/api/shares")
            .cookie(&self.admin)
            .send(&self.h.app)
            .await;
        res.json["shares"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["hits"].as_i64().unwrap())
            .collect()
    }

    fn admin_patch(&self, patch: UserPatch) {
        let id = users::by_email(&self.h.state.db, "admin@example.com")
            .unwrap()
            .unwrap()
            .0
            .id;
        users::update(&self.h.state.db, id, patch).unwrap();
    }
}

fn text(res: &Res) -> String {
    String::from_utf8_lossy(&res.body).into_owned()
}

#[tokio::test]
async fn page_is_app() {
    let f = fx().await;
    let id = f.file().await;
    let res = f.get(&format!("/s/{id}")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.header("content-type").starts_with("text/html"));
    assert_eq!(res.header("content-security-policy"), APP_CSP);
    assert_eq!(res.header("cache-control"), "no-cache");
    assert!(text(&res).starts_with("<!doctype html>"));
}

#[tokio::test]
async fn page_for_gone_id_is_app() {
    let f = fx().await;
    let res = f.get("/s/k7Qw3nTzAbCdEfGhIjKl-_").await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.header("content-type").starts_with("text/html"));
}

#[tokio::test]
async fn bad_id_404_plain() {
    let f = fx().await;
    for path in [
        "/s/short",
        "/s/k7Qw3nTzAbCdEfGhIjKl+_/info",
        "/s/short/info",
    ] {
        let res = f.get(path).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
        assert!(
            res.header("content-type").starts_with("text/plain"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn path_variants_404() {
    let f = fx().await;
    let id = f.file().await;
    for path in [
        format!("/s/{id}/"),
        format!("/S/{id}"),
        format!("/s//{id}"),
        format!("/s/{id}/nope"),
        format!("/s/{id}/info/"),
        "/s".to_string(),
        "/s/".to_string(),
    ] {
        let res = f.get(&path).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
        assert!(
            res.header("content-type").starts_with("text/plain"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn other_methods_404() {
    let f = fx().await;
    let id = f.file().await;
    for req in [
        Req::post(&format!("/s/{id}")),
        Req::delete(&format!("/s/{id}/raw")),
        Req::put(&format!("/s/{id}/info")),
    ] {
        let res = req.send(&f.h.app).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND);
        assert!(res.header("content-type").starts_with("text/plain"));
    }
}

#[tokio::test]
async fn info_file() {
    let f = fx().await;
    let id = f.file().await;
    let res = f.get(&format!("/s/{id}/info")).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["entry"]["name"], "trail-ridge.jpg");
    assert_eq!(res.json["entry"]["kind"], "file");
    assert_eq!(res.json["entry"]["size"], 10);
    assert_eq!(res.json["download"], true);
    assert!(res.json["expiresAt"].as_i64().unwrap() > cairn::now());
    assert_eq!(res.json["sharedBy"], "Someone");
}

#[tokio::test]
async fn info_folder_named_after_it() {
    let f = fx().await;
    let id = f.folder(true).await;
    let res = f.get(&format!("/s/{id}/info")).await;
    assert_eq!(res.json["entry"]["name"], "photos");
    assert_eq!(res.json["entry"]["kind"], "folder");
}

#[tokio::test]
async fn info_counts_hits() {
    let f = fx().await;
    let id = f.file().await;
    f.get(&format!("/s/{id}/info")).await;
    f.get(&format!("/s/{id}/info")).await;
    assert_eq!(f.hits().await, vec![2]);
    f.get(&format!("/s/{id}/raw")).await;
    assert_eq!(f.hits().await, vec![2]);
}

async fn assert_gone(f: &Fx, id: &str) {
    let res = f.get(&format!("/s/{id}/info")).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.json);
    assert_eq!(res.code(), "share_gone");
    assert_eq!(res.json["message"], GONE);
}

#[tokio::test]
async fn info_gone_reasons() {
    let f = fx().await;
    // Expired.
    let id = f.file().await;
    f.h.state
        .db
        .with(|c| c.execute("UPDATE shares SET expires_at = 1 WHERE id = ?1", [&id]))
        .unwrap();
    assert_gone(&f, &id).await;
    // Deleted.
    let id = f.file().await;
    Req::delete(&format!("/api/shares/{id}"))
        .cookie(&f.admin)
        .send(&f.h.app)
        .await;
    assert_gone(&f, &id).await;
    // Target removed.
    let id = f
        .share(json!({ "path": "photos/notes.txt", "expiresDays": 7, "download": true }))
        .await;
    std::fs::remove_file(f.dir.path().join("srv/photos/notes.txt")).unwrap();
    assert_gone(&f, &id).await;
    // Creator scope narrowed, then creator without share.
    let id = f.file().await;
    f.admin_patch(UserPatch {
        scope: Some("/photos/sub".into()),
        ..Default::default()
    });
    assert_gone(&f, &id).await;
    f.admin_patch(UserPatch {
        scope: Some("/".into()),
        perms: Some(Perms {
            read: true,
            write: true,
            share: false,
            admin: true,
        }),
        ..Default::default()
    });
    assert_gone(&f, &id).await;
}

#[tokio::test]
async fn raw_file() {
    let f = fx().await;
    let id = f.file().await;
    let res = f.get(&format!("/s/{id}/raw")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body, b"jpeg bytes");
    assert_eq!(res.header("content-type"), "image/jpeg");
    assert!(res.header("content-disposition").starts_with("inline"));
}

#[tokio::test]
async fn raw_headers_strict() {
    let f = fx().await;
    let file = f.file().await;
    let folder = f.folder(true).await;
    for path in [format!("/s/{file}/raw"), format!("/s/{folder}/zip")] {
        let res = f.get(&path).await;
        assert_eq!(res.status, StatusCode::OK, "{path}");
        assert_eq!(res.header("content-security-policy"), RAW_CSP, "{path}");
        assert_eq!(res.header("x-content-type-options"), "nosniff", "{path}");
        assert_eq!(res.header("cache-control"), "private, no-cache", "{path}");
    }
}

#[tokio::test]
async fn html_file_cannot_run() {
    let f = fx().await;
    let id = f
        .share(json!({ "path": "photos/page.html", "expiresDays": 7, "download": true }))
        .await;
    let res = f.get(&format!("/s/{id}/raw")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.header("content-security-policy").contains("sandbox"));
    assert_eq!(res.header("x-content-type-options"), "nosniff");
}

#[tokio::test]
async fn raw_range_and_etag() {
    let f = fx().await;
    let id = f.file().await;
    let res = Req::get(&format!("/s/{id}/raw"))
        .header("range", "bytes=0-3")
        .send(&f.h.app)
        .await;
    assert_eq!(res.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(res.body, b"jpeg");
    let tag = res.header("etag").to_string();
    let res = Req::get(&format!("/s/{id}/raw"))
        .header("if-none-match", &tag)
        .send(&f.h.app)
        .await;
    assert_eq!(res.status, StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn download_on_attachment() {
    let f = fx().await;
    let id = f.file().await;
    let res = f.get(&format!("/s/{id}/raw?download=1")).await;
    assert_eq!(
        res.header("content-disposition"),
        "attachment; filename*=UTF-8''trail-ridge.jpg"
    );
}

#[tokio::test]
async fn download_off_inline() {
    let f = fx().await;
    let id = f
        .share(
            json!({ "path": "photos/2026/trail-ridge.jpg", "expiresDays": 7, "download": false }),
        )
        .await;
    let res = f.get(&format!("/s/{id}/raw?download=1")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.header("content-disposition").starts_with("inline"));
    let info = f.get(&format!("/s/{id}/info")).await;
    assert_eq!(info.json["download"], false);
}

#[tokio::test]
async fn zip_download_off_403() {
    let f = fx().await;
    let id = f.folder(false).await;
    let res = f.get(&format!("/s/{id}/zip")).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.code(), "download_off");
    assert_eq!(res.json["message"], "Downloads are off for this link.");
}

#[tokio::test]
async fn zip_folder() {
    let f = fx().await;
    let id = f.folder(true).await;
    let res = f.get(&format!("/s/{id}/zip")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("content-type"), "application/zip");
    let body = text(&res);
    assert!(body.contains("notes.txt") && body.contains("sub/deep.txt"));
    let res = f.get(&format!("/s/{id}/zip/sub")).await;
    assert!(text(&res).contains("deep.txt"));
}

#[tokio::test]
async fn folder_list_and_nested() {
    let f = fx().await;
    let id = f.folder(true).await;
    let res = f.get(&format!("/s/{id}/list")).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let names: Vec<&str> = res.json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["2026", "sub", "notes.txt", "page.html"]);
    let res = f.get(&format!("/s/{id}/list/sub")).await;
    assert_eq!(res.json["path"], "sub");
    assert_eq!(res.json["entries"][0]["name"], "deep.txt");
    let res = f.get(&format!("/s/{id}/raw/sub/deep.txt")).await;
    assert_eq!(res.body, b"deep");
    let res = f.get(&format!("/s/{id}/raw")).await;
    assert_eq!(res.code(), "is_a_folder");
    let file = f.file().await;
    let res = f.get(&format!("/s/{file}/list")).await;
    assert_eq!(res.code(), "not_a_folder");
}

#[tokio::test]
async fn nested_cannot_escape() {
    let f = fx().await;
    let id = f.folder(true).await;
    for path in [
        format!("/s/{id}/raw/..%2Fsecret.txt"),
        format!("/s/{id}/raw/../secret.txt"),
        format!("/s/{id}/list/../.."),
        format!("/s/{id}/raw/%2E%2E/secret.txt"),
    ] {
        let res = f.get(&path).await;
        assert!(
            res.status == StatusCode::BAD_REQUEST || res.status == StatusCode::NOT_FOUND,
            "{path}: {}",
            res.status
        );
        assert_ne!(res.body, b"secret", "{path}");
    }
}

#[tokio::test]
async fn odd_names_through_share() {
    let f = fx().await;
    let photos = f.dir.path().join("srv/photos");
    std::fs::write(photos.join("Report #1 (50%) ✓ café.txt"), "report").unwrap();
    std::fs::write(photos.join(OsStr::from_bytes(b"caf\xe9.txt")), "latin").unwrap();
    let id = f.folder(true).await;
    let res = f.get(&format!("/s/{id}/list")).await;
    let entries = res.json["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["name"] == "Report #1 (50%) ✓ café.txt")
    );
    let raw = entries
        .iter()
        .find_map(|e| e["rawName"].as_str())
        .expect("a raw name");
    let res = f
        .get(&format!(
            "/s/{id}/raw/{}",
            enc("Report #1 (50%) ✓ café.txt")
        ))
        .await;
    assert_eq!(res.body, b"report");
    let res = f.get(&format!("/s/{id}/raw/{}", enc(raw))).await;
    assert_eq!(res.body, b"latin");
}

#[tokio::test]
async fn locked_without_cookie() {
    let f = fx().await;
    let id = f
        .share(
            json!({ "path": "photos", "expiresDays": 7, "password": "s3cret-pass", "download": true }),
        )
        .await;
    for path in ["info", "list", "raw/notes.txt", "zip"] {
        let res = f.get(&format!("/s/{id}/{path}")).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(res.code(), "share_locked", "{path}");
        assert_eq!(res.json["message"], "This link needs a password.");
    }
    assert_eq!(f.hits().await, vec![0]);
}

#[tokio::test]
async fn no_session_needed() {
    // Every request in `Fx::get` goes out without a cookie; this one checks a signed-out visitor sees the file.
    let f = fx().await;
    let id = f.file().await;
    let res = Req::get(&format!("/s/{id}/info")).send(&f.h.app).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["entry"]["name"], "trail-ridge.jpg");
    let me = Req::get("/api/me").send(&f.h.app).await;
    assert_eq!(me.json["user"], Value::Null);
}

async fn locked_link(f: &Fx) -> String {
    f.share(
        json!({ "path": "photos", "expiresDays": 7, "password": "s3cret-pass", "download": true }),
    )
    .await
}

fn unlock(id: &str, password: &str) -> Req {
    Req::post(&format!("/s/{id}/unlock")).json(json!({ "password": password }))
}

fn pair(res: &Res) -> String {
    res.cookie().expect("unlock cookie")
}

#[tokio::test]
async fn unlock_right() {
    let f = fx().await;
    let id = locked_link(&f).await;
    let res = unlock(&id, "s3cret-pass").send(&f.h.app).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.json);
    let set = res.set_cookie().unwrap();
    for part in [
        "cairn_share=".to_string(),
        format!("Path=/s/{id}"),
        "HttpOnly".into(),
        "SameSite=Lax".into(),
        "Max-Age=43200".into(),
    ] {
        assert!(set.contains(&part), "{part} in {set}");
    }
    assert!(!set.contains("Secure"), "{set}");
    let c = pair(&res);
    let res = Req::get(&format!("/s/{id}/info"))
        .cookie(&c)
        .send(&f.h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let res = Req::get(&format!("/s/{id}/raw/notes.txt"))
        .cookie(&c)
        .send(&f.h.app)
        .await;
    assert_eq!(res.body, b"some notes");
}

#[tokio::test]
async fn unlock_https_secure() {
    let (h, dir) = harness_with_root(&[("CAIRN_TRUSTED_PROXIES", "10.0.0.0/8")]);
    std::fs::write(dir.path().join("srv/notes.txt"), "n").unwrap();
    h.add_user("admin@example.com", Some(PW), true);
    let admin = h.login("admin@example.com", PW).await;
    let res = Req::post("/api/shares")
        .cookie(&admin)
        .json(json!({ "path": "notes.txt", "expiresDays": 7, "password": "s3cret-pass", "download": true }))
        .send(&h.app)
        .await;
    let id = res.json["share"]["id"].as_str().unwrap().to_string();
    let res = unlock(&id, "s3cret-pass")
        .peer("10.0.0.1:5000")
        .header("x-forwarded-proto", "https")
        .send(&h.app)
        .await;
    assert!(res.set_cookie().unwrap().ends_with("; Secure"));
}

#[tokio::test]
async fn unlock_wrong_is_403() {
    let f = fx().await;
    let id = locked_link(&f).await;
    let res = unlock(&id, "nope-wrong").send(&f.h.app).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.code(), "wrong_link_password");
    assert_eq!(res.json["message"], "That password is not right.");
    assert!(res.set_cookie().is_none());
}

#[tokio::test]
async fn unlock_needs_x_cairn() {
    let f = fx().await;
    let id = locked_link(&f).await;
    let res = unlock(&id, "s3cret-pass").no_csrf().send(&f.h.app).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.code(), "csrf");
}

#[tokio::test]
async fn unlock_unknown_field_400() {
    let f = fx().await;
    let id = locked_link(&f).await;
    let res = Req::post(&format!("/s/{id}/unlock"))
        .json(json!({ "password": "s3cret-pass", "keep": true }))
        .send(&f.h.app)
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unlock_gone_404() {
    let f = fx().await;
    let res = unlock("k7Qw3nTzAbCdEfGhIjKl-_", "s3cret-pass")
        .send(&f.h.app)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.code(), "share_gone");
}

#[tokio::test]
async fn unlock_open_link_204_no_cookie() {
    let f = fx().await;
    let id = f.file().await;
    let res = unlock(&id, "anything").send(&f.h.app).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(res.set_cookie().is_none());
}

#[tokio::test]
async fn cookie_for_other_link_refused() {
    let f = fx().await;
    let a = locked_link(&f).await;
    let b = locked_link(&f).await;
    let c = pair(&unlock(&a, "s3cret-pass").send(&f.h.app).await);
    let res = Req::get(&format!("/s/{b}/info"))
        .cookie(&c)
        .send(&f.h.app)
        .await;
    assert_eq!(res.code(), "share_locked");
}

#[tokio::test]
async fn password_change_ends_unlock() {
    let f = fx().await;
    let id = locked_link(&f).await;
    let c = pair(&unlock(&id, "s3cret-pass").send(&f.h.app).await);
    let patch = |body: Value| {
        Req::patch(&format!("/api/shares/{id}"))
            .cookie(&f.admin)
            .json(body)
    };
    patch(json!({ "password": "n3w-secret" }))
        .send(&f.h.app)
        .await;
    let res = Req::get(&format!("/s/{id}/info"))
        .cookie(&c)
        .send(&f.h.app)
        .await;
    assert_eq!(res.code(), "share_locked");
    patch(json!({ "password": null })).send(&f.h.app).await;
    let res = Req::get(&format!("/s/{id}/info")).send(&f.h.app).await;
    assert_eq!(res.status, StatusCode::OK);
}

#[tokio::test]
async fn restart_ends_unlock() {
    let f = fx().await;
    let id = locked_link(&f).await;
    let c = pair(&unlock(&id, "s3cret-pass").send(&f.h.app).await);
    let restarted =
        cairn::http::AppState::new((*f.h.state.config).clone(), cairn::db::Db::open_in_memory());
    let mut state = f.h.state.clone();
    state.share_key = restarted.share_key;
    let app = cairn::http::router(state);
    let res = Req::get(&format!("/s/{id}/info"))
        .cookie(&c)
        .send(&app)
        .await;
    assert_eq!(res.code(), "share_locked");
}

#[tokio::test]
async fn wrong_password_limit() {
    let f = fx().await;
    let id = locked_link(&f).await;
    let other = locked_link(&f).await;
    let ip = "203.0.113.5:4000";
    for _ in 0..10 {
        let res = unlock(&id, "wrong-one").peer(ip).send(&f.h.app).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN);
    }
    let res = unlock(&id, "s3cret-pass").peer(ip).send(&f.h.app).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(!res.header("retry-after").is_empty());
    let res = unlock(&id, "s3cret-pass")
        .peer("203.0.113.6:4000")
        .send(&f.h.app)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = unlock(&other, "s3cret-pass").peer(ip).send(&f.h.app).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn limit_counts_per_64() {
    let f = fx().await;
    let id = locked_link(&f).await;
    for i in 1..=10 {
        let peer = format!("[2001:db8:0:1::{i:x}]:4000");
        unlock(&id, "wrong-one").peer(&peer).send(&f.h.app).await;
    }
    let res = unlock(&id, "s3cret-pass")
        .peer("[2001:db8:0:1:ffff::1]:4000")
        .send(&f.h.app)
        .await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    let res = unlock(&id, "s3cret-pass")
        .peer("[2001:db8:0:2::1]:4000")
        .send(&f.h.app)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
}
