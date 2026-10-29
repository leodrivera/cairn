//! Server settings and server information, through the real router.

mod common;

use axum::http::StatusCode;
use common::{Harness, Req, harness};
use serde_json::json;

const PW: &str = "Abcdef1!";

async fn admin(h: &Harness) -> String {
    h.add_user("admin@example.com", Some(PW), true);
    h.login("admin@example.com", PW).await
}

async fn reader(h: &Harness) -> String {
    h.add_user("ana@example.com", Some(PW), false);
    h.login("ana@example.com", PW).await
}

#[tokio::test]
async fn config_has_theme_thumbnails_edit_max() {
    let h = harness(&[("CAIRN_EDIT_MAX", "8")]);
    let res = Req::get("/api/config").send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["theme"], json!(null));
    assert_eq!(res.json["thumbnails"], json!(false));
    assert_eq!(res.json["editMax"], json!(8));
    assert_eq!(res.json["providers"], json!([]));
}

#[tokio::test]
async fn put_settings_admin_only() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let r = reader(&h).await;
    let body = json!({ "theme": "light", "thumbnails": true });
    let res = Req::put("/api/settings")
        .cookie(&r)
        .json(body.clone())
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.code(), "forbidden");
    let res = Req::put("/api/settings")
        .cookie(&a)
        .json(body.clone())
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["settings"], body);
    let cfg = Req::get("/api/config").send(&h.app).await;
    assert_eq!(cfg.json["theme"], json!("light"));
    assert_eq!(cfg.json["thumbnails"], json!(true));
}

#[tokio::test]
async fn put_settings_clears_theme() {
    let h = harness(&[]);
    let a = admin(&h).await;
    for theme in [json!("dark"), json!(null)] {
        let res = Req::put("/api/settings")
            .cookie(&a)
            .json(json!({ "theme": theme, "thumbnails": false }))
            .send(&h.app)
            .await;
        assert_eq!(res.status, StatusCode::OK);
    }
    let cfg = Req::get("/api/config").send(&h.app).await;
    assert_eq!(cfg.json["theme"], json!(null));
}

#[tokio::test]
async fn put_settings_bad_theme() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let res = Req::put("/api/settings")
        .cookie(&a)
        .json(json!({ "theme": "blue", "thumbnails": false }))
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "invalid");
}

#[tokio::test]
async fn put_settings_needs_x_cairn() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let res = Req::put("/api/settings")
        .cookie(&a)
        .no_csrf()
        .json(json!({ "theme": null, "thumbnails": true }))
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.code(), "csrf");
}

#[tokio::test]
async fn put_settings_unknown_field_400() {
    let h = harness(&[]);
    let a = admin(&h).await;
    let res = Req::put("/api/settings")
        .cookie(&a)
        .json(json!({ "theme": null, "thumbnails": true, "extra": 1 }))
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn server_info_admin_only() {
    let h = harness(&[
        ("CAIRN_TRUSTED_PROXIES", "172.16.0.0/12"),
        ("CAIRN_UPLOAD_MAX", "1000"),
    ]);
    let a = admin(&h).await;
    let r = reader(&h).await;
    let res = Req::get("/api/server").cookie(&r).send(&h.app).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = Req::get("/api/server").cookie(&a).send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["root"], json!("/srv"));
    assert_eq!(res.json["publicUrl"], json!(null));
    assert_eq!(res.json["addr"], json!("0.0.0.0:8080"));
    assert_eq!(res.json["authMethods"], json!(["password"]));
    assert_eq!(res.json["captcha"], json!("none"));
    assert_eq!(res.json["uploadMax"], json!(1000));
    assert_eq!(res.json["editMax"], json!(2097152));
    assert_eq!(res.json["threads"], json!(1));
    assert_eq!(res.json["trustedProxies"], json!(["172.16.0.0/12"]));
    assert_eq!(res.json["log"], json!("info"));
}

#[tokio::test]
async fn server_info_has_no_secrets() {
    let h = harness(&[
        ("CAIRN_PUBLIC_URL", "https://files.example.com"),
        ("CAIRN_CAPTCHA", "recaptcha"),
        ("CAIRN_CAPTCHA_SITE_KEY", "site-key-placeholder"),
        ("CAIRN_CAPTCHA_SECRET", "s3cr3t-placeholder"),
    ]);
    let a = h.session_for(&h.add_user("admin@example.com", Some(PW), true));
    let res = Req::get("/api/server").cookie(&a).send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["captcha"], json!("recaptcha"));
    assert_eq!(res.json["publicUrl"], json!("https://files.example.com/"));
    assert!(!String::from_utf8_lossy(&res.body).contains("s3cr3t"));
}
