//! Share link management: `/api/shares`.

mod common;

use axum::http::StatusCode;
use cairn::users::Perms;
use common::{Harness, Req, harness_with_root};
use serde_json::{Value, json};

const PW: &str = "Abcdef1!";
const PUBLIC: (&str, &str) = ("CAIRN_PUBLIC_URL", "https://files.example.com");
const SHARER: Perms = Perms {
    read: true,
    write: false,
    share: true,
    admin: false,
};
const READER: Perms = Perms {
    read: true,
    write: false,
    share: false,
    admin: false,
};

/// `srv/photos/2026/trail-ridge.jpg`, `srv/maria/docs/a.txt`, `srv/other.txt`.
fn tree(dir: &tempfile::TempDir) {
    let srv = dir.path().join("srv");
    std::fs::create_dir_all(srv.join("photos/2026")).unwrap();
    std::fs::create_dir_all(srv.join("maria/docs")).unwrap();
    std::fs::write(srv.join("photos/2026/trail-ridge.jpg"), "jpg").unwrap();
    std::fs::write(srv.join("maria/docs/a.txt"), "a").unwrap();
    std::fs::write(srv.join("other.txt"), "o").unwrap();
}

async fn setup(extra: &[(&str, &str)]) -> (Harness, tempfile::TempDir) {
    let (h, dir) = harness_with_root(extra);
    tree(&dir);
    (h, dir)
}

async fn sharer(h: &Harness, email: &str, scope: &str) -> String {
    h.add_user_scoped(email, PW, scope, SHARER);
    h.login(email, PW).await
}

async fn admin(h: &Harness) -> String {
    h.add_user("admin@example.com", Some(PW), true);
    h.login("admin@example.com", PW).await
}

async fn create(h: &Harness, cookie: &str, body: Value) -> common::Res {
    Req::post("/api/shares")
        .cookie(cookie)
        .json(body)
        .send(&h.app)
        .await
}

fn file_body() -> Value {
    json!({ "path": "photos/2026/trail-ridge.jpg", "expiresDays": 7, "password": null, "download": true })
}

async fn list(h: &Harness, cookie: &str) -> Vec<Value> {
    let res = Req::get("/api/shares").cookie(cookie).send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    res.json["shares"].as_array().unwrap().clone()
}

#[tokio::test]
async fn create_file_link() {
    let (h, _d) = setup(&[PUBLIC]).await;
    let c = sharer(&h, "ana@example.com", "/").await;
    let res = create(&h, &c, file_body()).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let s = &res.json["share"];
    let id = s["id"].as_str().unwrap();
    assert_eq!(id.len(), 22);
    assert_eq!(s["url"], format!("https://files.example.com/s/{id}"));
    let expires = s["expiresAt"].as_i64().unwrap();
    assert!(
        (expires - (cairn::now() + 7 * 86_400)).abs() <= 5,
        "{expires}"
    );
    assert_eq!(s["hasPassword"], false);
    assert_eq!(s["download"], true);
    assert_eq!(s["hits"], 0);
    assert_eq!(s["path"], "/photos/2026/trail-ridge.jpg");
    assert_eq!(s["name"], "trail-ridge.jpg");
    assert_eq!(s["owner"]["name"], "Someone");
}

#[tokio::test]
async fn url_null_without_public_url() {
    let (h, _d) = setup(&[]).await;
    let c = sharer(&h, "ana@example.com", "/").await;
    let res = create(&h, &c, file_body()).await;
    assert_eq!(res.status, StatusCode::CREATED);
    assert_eq!(res.json["share"]["url"], Value::Null);
}

#[tokio::test]
async fn create_needs_share() {
    let (h, _d) = setup(&[]).await;
    h.add_user_scoped("guest@example.com", PW, "/", READER);
    let c = h.login("guest@example.com", PW).await;
    let res = create(&h, &c, file_body()).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.code(), "forbidden");
    assert_eq!(
        res.json["message"],
        "You do not have permission to do this here."
    );
    let res = Req::post("/api/shares")
        .json(file_body())
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.code(), "signed_out");
}

#[tokio::test]
async fn create_in_scope_only() {
    let (h, _d) = setup(&[]).await;
    let c = sharer(&h, "maria@example.com", "/maria").await;
    let res = create(
        &h,
        &c,
        json!({ "path": "../other.txt", "expiresDays": 7, "download": true }),
    )
    .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.json);
    let res = create(
        &h,
        &c,
        json!({ "path": "other.txt", "expiresDays": 7, "download": true }),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.json);
    let res = create(
        &h,
        &c,
        json!({ "path": "docs/a.txt", "expiresDays": 7, "download": true }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
}

#[tokio::test]
async fn create_missing_is_404() {
    let (h, _d) = setup(&[]).await;
    let c = sharer(&h, "ana@example.com", "/").await;
    let res = create(
        &h,
        &c,
        json!({ "path": "nope.txt", "expiresDays": 7, "download": true }),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_bad_expiry() {
    let (h, _d) = setup(&[]).await;
    let c = sharer(&h, "ana@example.com", "/").await;
    let res = create(
        &h,
        &c,
        json!({ "path": "other.txt", "expiresDays": 2, "download": true }),
    )
    .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "invalid");
    assert_eq!(
        res.json["message"],
        "Expiry must be 1, 7 or 30 days, or never."
    );
}

#[tokio::test]
async fn create_password_hashed() {
    let (h, _d) = setup(&[]).await;
    let c = sharer(&h, "ana@example.com", "/").await;
    let res = create(
        &h,
        &c,
        json!({ "path": "other.txt", "expiresDays": null, "password": "s3cret", "download": true }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(res.json["share"]["hasPassword"], true);
    assert_eq!(res.json["share"]["expiresAt"], Value::Null);
    let id = res.json["share"]["id"].as_str().unwrap();
    let row = cairn::shares::get(&h.state.db, id).unwrap().unwrap();
    let hash = row.password_hash.unwrap();
    assert!(hash.starts_with("$argon2id$"), "{hash}");
    assert!(!hash.contains("s3cret"));
}

#[tokio::test]
async fn create_bad_password() {
    let (h, _d) = setup(&[]).await;
    let c = sharer(&h, "ana@example.com", "/").await;
    for pw in [String::new(), "x".repeat(257)] {
        let res = create(
            &h,
            &c,
            json!({ "path": "other.txt", "expiresDays": 7, "password": pw, "download": true }),
        )
        .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            res.json["message"],
            "The link password needs 1 to 256 characters."
        );
    }
}

#[tokio::test]
async fn create_unknown_field_refused() {
    let (h, _d) = setup(&[]).await;
    let c = sharer(&h, "ana@example.com", "/").await;
    let res = create(
        &h,
        &c,
        json!({ "path": "other.txt", "expiresDays": 7, "download": true, "hits": 99 }),
    )
    .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_needs_x_cairn() {
    let (h, _d) = setup(&[]).await;
    let c = sharer(&h, "ana@example.com", "/").await;
    let res = Req::post("/api/shares")
        .cookie(&c)
        .json(file_body())
        .no_csrf()
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.code(), "csrf");
}

#[tokio::test]
async fn list_own_only() {
    let (h, _d) = setup(&[]).await;
    let a = sharer(&h, "ana@example.com", "/").await;
    let b = sharer(&h, "bo@example.com", "/").await;
    create(&h, &a, file_body()).await;
    create(
        &h,
        &b,
        json!({ "path": "other.txt", "expiresDays": 7, "download": true }),
    )
    .await;
    let mine = list(&h, &a).await;
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0]["name"], "trail-ridge.jpg");
    assert_eq!(list(&h, &b).await.len(), 1);
}

#[tokio::test]
async fn admin_lists_all() {
    let (h, _d) = setup(&[]).await;
    let a = sharer(&h, "ana@example.com", "/").await;
    let adm = admin(&h).await;
    create(&h, &a, file_body()).await;
    create(
        &h,
        &adm,
        json!({ "path": "other.txt", "expiresDays": 7, "download": true }),
    )
    .await;
    assert_eq!(list(&h, &adm).await.len(), 2);
}

#[tokio::test]
async fn list_path_relative_to_viewer() {
    let (h, _d) = setup(&[]).await;
    let m = sharer(&h, "maria@example.com", "/maria").await;
    let adm = admin(&h).await;
    create(
        &h,
        &m,
        json!({ "path": "docs/a.txt", "expiresDays": 7, "download": true }),
    )
    .await;
    assert_eq!(list(&h, &m).await[0]["path"], "/docs/a.txt");
    assert_eq!(list(&h, &adm).await[0]["path"], "/maria/docs/a.txt");
}

#[tokio::test]
async fn list_hides_expired() {
    let (h, _d) = setup(&[]).await;
    let c = sharer(&h, "ana@example.com", "/").await;
    let id = create(&h, &c, file_body()).await.json["share"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.state
        .db
        .with(|c| c.execute("UPDATE shares SET expires_at = 1 WHERE id = ?1", [&id]))
        .unwrap();
    assert!(list(&h, &c).await.is_empty());
}

async fn patch(h: &Harness, cookie: &str, id: &str, body: Value) -> common::Res {
    Req::patch(&format!("/api/shares/{id}"))
        .cookie(cookie)
        .json(body)
        .send(&h.app)
        .await
}

#[tokio::test]
async fn patch_fields() {
    let (h, _d) = setup(&[]).await;
    let c = sharer(&h, "ana@example.com", "/").await;
    let id = create(&h, &c, file_body()).await.json["share"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = patch(&h, &c, &id, json!({ "expiresDays": null })).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["share"]["expiresAt"], Value::Null);
    let res = patch(&h, &c, &id, json!({ "expiresDays": 30 })).await;
    let at = res.json["share"]["expiresAt"].as_i64().unwrap();
    assert!((at - (cairn::now() + 30 * 86_400)).abs() <= 5);
    let res = patch(&h, &c, &id, json!({ "password": "s3cret" })).await;
    assert_eq!(res.json["share"]["hasPassword"], true);
    let res = patch(&h, &c, &id, json!({ "password": null })).await;
    assert_eq!(res.json["share"]["hasPassword"], false);
    let res = patch(&h, &c, &id, json!({ "download": false })).await;
    assert_eq!(res.json["share"]["download"], false);
    let res = patch(&h, &c, &id, json!({ "expiresDays": 3 })).await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    let res = patch(&h, &c, &id, json!({ "password": "" })).await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn patch_other_user_404() {
    let (h, _d) = setup(&[]).await;
    let a = sharer(&h, "ana@example.com", "/").await;
    let b = sharer(&h, "bo@example.com", "/").await;
    let adm = admin(&h).await;
    let id = create(&h, &a, file_body()).await.json["share"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = patch(&h, &b, &id, json!({ "download": false })).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.code(), "not_found");
    assert_eq!(res.json["message"], "No such share link.");
    let res = patch(&h, &adm, &id, json!({ "download": false })).await;
    assert_eq!(res.status, StatusCode::OK);
}

#[tokio::test]
async fn delete_own() {
    let (h, _d) = setup(&[]).await;
    let c = sharer(&h, "ana@example.com", "/").await;
    let id = create(&h, &c, file_body()).await.json["share"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = Req::delete(&format!("/api/shares/{id}"))
        .cookie(&c)
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(list(&h, &c).await.is_empty());
}

#[tokio::test]
async fn delete_other_user_404() {
    let (h, _d) = setup(&[]).await;
    let a = sharer(&h, "ana@example.com", "/").await;
    let b = sharer(&h, "bo@example.com", "/").await;
    let adm = admin(&h).await;
    let id = create(&h, &a, file_body()).await.json["share"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let del = |c: String| {
        let id = id.clone();
        let app = h.app.clone();
        async move {
            Req::delete(&format!("/api/shares/{id}"))
                .cookie(&c)
                .send(&app)
                .await
        }
    };
    assert_eq!(del(b).await.status, StatusCode::NOT_FOUND);
    assert_eq!(del(adm).await.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn user_delete_removes_links() {
    let (h, _d) = setup(&[]).await;
    let a = sharer(&h, "ana@example.com", "/").await;
    let adm = admin(&h).await;
    create(&h, &a, file_body()).await;
    assert_eq!(list(&h, &adm).await.len(), 1);
    let ana = cairn::users::by_email(&h.state.db, "ana@example.com")
        .unwrap()
        .unwrap()
        .0;
    let res = Req::delete(&format!("/api/users/{}", ana.id))
        .cookie(&adm)
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.json);
    assert!(list(&h, &adm).await.is_empty());
}
