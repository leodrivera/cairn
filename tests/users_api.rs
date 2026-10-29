//! Users admin API, through the real router.

mod common;

use axum::http::StatusCode;
use common::{Harness, Req, harness};
use serde_json::{Value, json};

const PW: &str = "Abcdef1!";

async fn admin(h: &Harness) -> String {
    h.add_user("admin@example.com", Some(PW), true);
    h.login("admin@example.com", PW).await
}

fn perms(read: bool, write: bool, share: bool, admin: bool) -> Value {
    json!({ "read": read, "write": write, "share": share, "admin": admin })
}

async fn create(h: &Harness, cookie: &str, email: &str, password: Option<&str>) -> Value {
    let mut body = json!({ "email": email, "name": "Maria", "scope": "/maria", "permissions": perms(true, true, false, false) });
    if let Some(p) = password {
        body["password"] = json!(p);
    }
    Req::post("/api/users")
        .cookie(cookie)
        .json(body)
        .send(&h.app)
        .await
        .json
}

#[tokio::test]
async fn non_admin_forbidden() {
    let h = harness(&[]);
    admin(&h).await;
    h.add_user("ana@example.com", Some(PW), false);
    let cookie = h.login("ana@example.com", PW).await;
    let res = Req::get("/api/users").cookie(&cookie).send(&h.app).await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
}

#[tokio::test]
async fn signed_out_forbidden() {
    let h = harness(&[]);
    let res = Req::get("/api/users").send(&h.app).await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::FORBIDDEN, "signed_out")
    );
}

#[tokio::test]
async fn create_list_update_delete() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let res = Req::post("/api/users")
        .cookie(&a)
        .json(json!({ "email": "Maria@Example.com", "name": "Maria", "password": PW, "scope": "/maria",
                      "permissions": perms(false, true, false, false) }))
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let id = res.json["user"]["id"].as_i64().unwrap();
    assert_eq!(res.json["user"]["email"], "maria@example.com");
    assert_eq!(
        res.json["user"]["permissions"],
        perms(true, true, false, false)
    );

    let list = Req::get("/api/users").cookie(&a).send(&h.app).await;
    assert_eq!(list.json["users"].as_array().unwrap().len(), 2);

    let res = Req::patch(&format!("/api/users/{id}"))
        .cookie(&a)
        .json(json!({ "name": "Maria S." }))
        .send(&h.app)
        .await;
    assert_eq!(res.json["user"]["name"], "Maria S.");

    let res = Req::delete(&format!("/api/users/{id}"))
        .cookie(&a)
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let missing = Req::delete(&format!("/api/users/{id}"))
        .cookie(&a)
        .send(&h.app)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_needs_password_without_sso() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let body = json!({ "email": "m@example.com", "name": "M", "scope": "/", "permissions": perms(true, false, false, false) });
    let res = Req::post("/api/users")
        .cookie(&a)
        .json(body)
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::BAD_REQUEST, "password_required")
    );
    let weak = create(&h, &a, "w@example.com", Some("weak")).await;
    assert_eq!(weak["error"], "weak_password");
}

#[tokio::test]
async fn scope_change_ends_sessions() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let id = create(&h, &a, "maria@example.com", Some(PW)).await["user"]["id"]
        .as_i64()
        .unwrap();
    let m = h.login("maria@example.com", PW).await;
    Req::patch(&format!("/api/users/{id}"))
        .cookie(&a)
        .json(json!({ "scope": "/other" }))
        .send(&h.app)
        .await;
    assert_eq!(
        Req::get("/api/me").cookie(&m).send(&h.app).await.json,
        json!({ "user": null })
    );
}

#[tokio::test]
async fn name_change_keeps_sessions() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let id = create(&h, &a, "maria@example.com", Some(PW)).await["user"]["id"]
        .as_i64()
        .unwrap();
    let m = h.login("maria@example.com", PW).await;
    Req::patch(&format!("/api/users/{id}"))
        .cookie(&a)
        .json(json!({ "name": "M" }))
        .send(&h.app)
        .await;
    assert_eq!(
        Req::get("/api/me").cookie(&m).send(&h.app).await.json["user"]["name"],
        "M"
    );
}

#[tokio::test]
async fn email_change_logs_and_ends_sessions() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let id = create(&h, &a, "maria@example.com", Some(PW)).await["user"]["id"]
        .as_i64()
        .unwrap();
    let m = h.login("maria@example.com", PW).await;
    let res = Req::patch(&format!("/api/users/{id}"))
        .cookie(&a)
        .json(json!({ "email": "ms@example.com" }))
        .send(&h.app)
        .await;
    assert_eq!(res.json["user"]["email"], "ms@example.com");
    assert_eq!(
        Req::get("/api/me").cookie(&m).send(&h.app).await.json,
        json!({ "user": null })
    );
    h.login("ms@example.com", PW).await;
}

#[tokio::test]
async fn own_email_refused() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let me = Req::get("/api/me").cookie(&a).send(&h.app).await.json["user"]["id"]
        .as_i64()
        .unwrap();
    let res = Req::patch(&format!("/api/users/{me}"))
        .cookie(&a)
        .json(json!({ "email": "boss@example.com" }))
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::BAD_REQUEST, "own_email")
    );
}

#[tokio::test]
async fn last_admin_delete_409() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let me = Req::get("/api/me").cookie(&a).send(&h.app).await.json["user"]["id"]
        .as_i64()
        .unwrap();
    let res = Req::delete(&format!("/api/users/{me}"))
        .cookie(&a)
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::CONFLICT, "last_admin")
    );
}

#[tokio::test]
async fn last_admin_demote_409() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let me = Req::get("/api/me").cookie(&a).send(&h.app).await.json["user"]["id"]
        .as_i64()
        .unwrap();
    let res = Req::patch(&format!("/api/users/{me}"))
        .cookie(&a)
        .json(json!({ "permissions": perms(true, true, true, false) }))
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::CONFLICT, "last_admin")
    );
}

#[tokio::test]
async fn reset_password_ends_sessions() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let id = create(&h, &a, "maria@example.com", Some(PW)).await["user"]["id"]
        .as_i64()
        .unwrap();
    let m = h.login("maria@example.com", PW).await;
    let res = Req::put(&format!("/api/users/{id}/password"))
        .cookie(&a)
        .json(json!({ "password": "Ghijkl2?" }))
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.json.clone()),
        (StatusCode::OK, json!({ "ok": true }))
    );
    assert_eq!(
        Req::get("/api/me").cookie(&m).send(&h.app).await.json,
        json!({ "user": null })
    );
    h.login("maria@example.com", "Ghijkl2?").await;
}

const OIDC_ON: &[(&str, &str)] = &[
    ("CAIRN_AUTH_METHODS", "password,oidc"),
    ("CAIRN_PUBLIC_URL", "https://files.example.com"),
    ("CAIRN_OIDC_PROVIDERS", "google"),
    ("CAIRN_OIDC_GOOGLE_ISSUER", "https://accounts.google.com"),
    ("CAIRN_OIDC_GOOGLE_CLIENT_ID", "id"),
    ("CAIRN_OIDC_GOOGLE_CLIENT_SECRET", "s"),
];

#[tokio::test]
async fn remove_password_only_with_sso() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let id = create(&h, &a, "maria@example.com", Some(PW)).await["user"]["id"]
        .as_i64()
        .unwrap();
    let res = Req::delete(&format!("/api/users/{id}/password"))
        .cookie(&a)
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::CONFLICT, "password_only")
    );

    let h = harness(OIDC_ON);
    let a = admin(&h).await;
    let id = create(&h, &a, "maria@example.com", Some(PW)).await["user"]["id"]
        .as_i64()
        .unwrap();
    let m = h.login("maria@example.com", PW).await;
    let res = Req::delete(&format!("/api/users/{id}/password"))
        .cookie(&a)
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(
        Req::get("/api/me").cookie(&m).send(&h.app).await.json,
        json!({ "user": null })
    );
    let created = create(&h, &a, "sso@example.com", None).await;
    assert_eq!(created["user"]["hasPassword"], false);
}
