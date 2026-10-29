//! Sign-in, sessions and profile, through the real router.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::{Req, harness};
use serde_json::json;

const PW: &str = "Abcdef1!";

fn login_body(email: &str, password: &str) -> serde_json::Value {
    json!({ "email": email, "password": password, "keep": false })
}

#[tokio::test]
async fn config_signed_out() {
    let h = harness(&[]);
    let res = Req::get("/api/config").send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["authMethods"], json!(["password"]));
    assert_eq!(res.json["captcha"], json!(null));
    assert_eq!(res.json["providers"], json!([]));
    assert_eq!(res.json["setupNeeded"], json!(true));
    h.add_user("admin@example.com", Some(PW), true);
    let res = Req::get("/api/config").send(&h.app).await;
    assert!(res.json.get("setupNeeded").is_none(), "{}", res.json);
}

#[tokio::test]
async fn me_signed_out_is_200_null() {
    let h = harness(&[]);
    let res = Req::get("/api/me").send(&h.app).await;
    assert_eq!(
        (res.status, res.json.clone()),
        (StatusCode::OK, json!({ "user": null }))
    );
}

#[tokio::test]
async fn login_ok_sets_cookie() {
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    let res = Req::post("/api/auth/login")
        .json(login_body("ana@example.com", PW))
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["user"]["email"], "ana@example.com");
    let c = res.set_cookie().unwrap();
    assert!(c.starts_with("cairn_session="), "{c}");
    for part in ["HttpOnly", "SameSite=Lax", "Path=/"] {
        assert!(c.contains(part), "{part} missing in {c}");
    }
    assert!(!c.contains("Max-Age") && !c.contains("Secure"), "{c}");

    let keep = json!({ "email": "ana@example.com", "password": PW, "keep": true });
    let res = Req::post("/api/auth/login").json(keep).send(&h.app).await;
    assert!(res.set_cookie().unwrap().contains("Max-Age=2592000"));
}

#[tokio::test]
async fn login_https_cookie() {
    let h = harness(&[("CAIRN_TRUSTED_PROXIES", "10.0.0.0/8")]);
    h.add_user("ana@example.com", Some(PW), false);
    let res = Req::post("/api/auth/login")
        .json(login_body("ana@example.com", PW))
        .peer("10.0.0.1:5000")
        .header("x-forwarded-proto", "https")
        .send(&h.app)
        .await;
    let c = res.set_cookie().unwrap();
    assert!(
        c.starts_with("__Host-cairn_session=") && c.contains("Secure"),
        "{c}"
    );
}

#[tokio::test]
async fn login_email_normalized() {
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    let res = Req::post("/api/auth/login")
        .json(login_body("  Ana@Example.COM ", PW))
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
}

const WRONG: &str = "Wrong e-mail or password.";

#[tokio::test]
async fn wrong_password_401() {
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    let res = Req::post("/api/auth/login")
        .json(login_body("ana@example.com", "Wrong1!x"))
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        res.json,
        json!({ "error": "wrong_credentials", "message": WRONG })
    );
}

#[tokio::test]
async fn unknown_email_401_same_body() {
    let h = harness(&[]);
    let res = Req::post("/api/auth/login")
        .json(login_body("nobody@example.com", PW))
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        res.json,
        json!({ "error": "wrong_credentials", "message": WRONG })
    );
}

#[tokio::test]
async fn oauth_only_user_401_same_body() {
    let h = harness(&[]);
    h.add_user("sso@example.com", None, false);
    let res = Req::post("/api/auth/login")
        .json(login_body("sso@example.com", PW))
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        res.json,
        json!({ "error": "wrong_credentials", "message": WRONG })
    );
}

#[tokio::test]
async fn login_needs_csrf_header() {
    let h = harness(&[]);
    let res = Req::post("/api/auth/login")
        .json(login_body("a@example.com", PW))
        .no_csrf()
        .send(&h.app)
        .await;
    assert_eq!((res.status, res.code()), (StatusCode::FORBIDDEN, "csrf"));
}

#[tokio::test]
async fn unknown_field_400() {
    let h = harness(&[]);
    let body = json!({ "email": "a@example.com", "password": PW, "keep": false, "admin": true });
    let res = Req::post("/api/auth/login").json(body).send(&h.app).await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::BAD_REQUEST, "bad_request")
    );
    let res = Req::post("/api/auth/login")
        .raw_json("{not json")
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::BAD_REQUEST, "bad_request")
    );
}

#[tokio::test]
async fn rate_limited_after_five() {
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    for _ in 0..5 {
        let res = Req::post("/api/auth/login")
            .json(login_body("ana@example.com", "Wrong1!x"))
            .send(&h.app)
            .await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    }
    let res = Req::post("/api/auth/login")
        .json(login_body("ana@example.com", PW))
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::TOO_MANY_REQUESTS, "too_many")
    );
    let wait: i64 = res.headers["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((899..=900).contains(&wait), "{wait}");
}

async fn fake_vendor(reply: &'static str) -> String {
    let app = axum::Router::new().route(
        "/verify",
        axum::routing::post(move || async move { ([("content-type", "application/json")], reply) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/verify", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

const CAPTCHA_ON: &[(&str, &str)] = &[
    ("CAIRN_CAPTCHA", "recaptcha"),
    ("CAIRN_PUBLIC_URL", "https://files.example.com"),
    ("CAIRN_CAPTCHA_SITE_KEY", "site-key"),
    ("CAIRN_CAPTCHA_SECRET", "secret"),
];

#[tokio::test]
async fn captcha_missing_400() {
    let url = fake_vendor(r#"{"success":true,"hostname":"files.example.com"}"#).await;
    let h = harness(CAPTCHA_ON).with_captcha_url(url);
    h.add_user("ana@example.com", Some(PW), false);
    let res = Req::post("/api/auth/login")
        .json(login_body("ana@example.com", PW))
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::BAD_REQUEST, "captcha_missing")
    );
    let config = Req::get("/api/config").send(&h.app).await;
    assert_eq!(
        config.json["captcha"],
        json!({ "provider": "recaptcha", "siteKey": "site-key" })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn captcha_ok_then_login() {
    let url = fake_vendor(r#"{"success":true,"hostname":"files.example.com"}"#).await;
    let h = harness(CAPTCHA_ON).with_captcha_url(url);
    h.add_user("ana@example.com", Some(PW), false);
    let body =
        json!({ "email": "ana@example.com", "password": PW, "keep": false, "captchaToken": "tok" });
    let res = Req::post("/api/auth/login").json(body).send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
}

#[tokio::test]
async fn me_with_cookie() {
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    let cookie = h.login("ana@example.com", PW).await;
    let res = Req::get("/api/me").cookie(&cookie).send(&h.app).await;
    assert_eq!(res.json["user"]["email"], "ana@example.com");
    assert_eq!(res.json["user"]["hasPassword"], true);
    assert_eq!(res.json["method"], "password");
    assert!(res.json.get("security").is_none());
}

#[tokio::test]
async fn logout_ends_session() {
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    let cookie = h.login("ana@example.com", PW).await;
    let res = Req::post("/api/auth/logout")
        .cookie(&cookie)
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.json.clone()),
        (StatusCode::OK, json!({ "next": null }))
    );
    assert!(res.set_cookie().unwrap().contains("Max-Age=0"));
    let me = Req::get("/api/me").cookie(&cookie).send(&h.app).await;
    assert_eq!(me.json, json!({ "user": null }));
}

#[tokio::test]
async fn patch_me_preferences() {
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    let cookie = h.login("ana@example.com", PW).await;
    let res = Req::patch("/api/me")
        .cookie(&cookie)
        .json(json!({ "name": "Ana", "lang": "pt-BR", "theme": "light", "view": "gallery" }))
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["user"]["name"], "Ana");
    assert_eq!(res.json["user"]["lang"], "pt-BR");
    let res = Req::patch("/api/me")
        .cookie(&cookie)
        .json(json!({ "theme": null }))
        .send(&h.app)
        .await;
    assert_eq!(res.json["user"]["theme"], json!(null));
    let res = Req::patch("/api/me")
        .cookie(&cookie)
        .json(json!({ "theme": "blue" }))
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::BAD_REQUEST, "invalid")
    );
    let signed_out = Req::patch("/api/me")
        .json(json!({ "name": "X" }))
        .send(&h.app)
        .await;
    assert_eq!(
        (signed_out.status, signed_out.code()),
        (StatusCode::FORBIDDEN, "signed_out")
    );
}

#[tokio::test]
async fn change_password_ends_other_sessions() {
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    let a = h.login("ana@example.com", PW).await;
    let b = h.login("ana@example.com", PW).await;
    let res = Req::put("/api/me/password")
        .cookie(&a)
        .json(json!({ "current": PW, "next": "Ghijkl2?" }))
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.json.clone()),
        (StatusCode::OK, json!({ "ok": true }))
    );
    assert_eq!(
        Req::get("/api/me").cookie(&a).send(&h.app).await.json["user"]["email"],
        "ana@example.com"
    );
    assert_eq!(
        Req::get("/api/me").cookie(&b).send(&h.app).await.json,
        json!({ "user": null })
    );
    h.login("ana@example.com", "Ghijkl2?").await;
}

#[tokio::test]
async fn change_password_wrong_current_400_not_401() {
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    let a = h.login("ana@example.com", PW).await;
    let res = Req::put("/api/me/password")
        .cookie(&a)
        .json(json!({ "current": "Nope123!", "next": "Ghijkl2?" }))
        .send(&h.app)
        .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::BAD_REQUEST, "wrong_password")
    );
    let weak = Req::put("/api/me/password")
        .cookie(&a)
        .json(json!({ "current": PW, "next": "weak" }))
        .send(&h.app)
        .await;
    assert_eq!(
        (weak.status, weak.code()),
        (StatusCode::BAD_REQUEST, "weak_password")
    );
}

#[tokio::test]
async fn admin_sees_captcha_notice() {
    let h = harness(&[]);
    h.add_user("admin@example.com", Some(PW), true);
    let cookie = h.login("admin@example.com", PW).await;
    let res = Req::get("/api/me").cookie(&cookie).send(&h.app).await;
    assert_eq!(res.json["security"], json!({ "captchaDisabled": true }));
}

#[tokio::test]
async fn sessions_are_not_shared_across_logins() {
    let _ = Duration::ZERO;
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    let a = h.login("ana@example.com", PW).await;
    let b = h.login("ana@example.com", PW).await;
    assert_ne!(a, b);
}

#[tokio::test]
async fn plain_cookie_ignored_over_https() {
    let h = harness(&[("CAIRN_TRUSTED_PROXIES", "10.0.0.0/8")]);
    h.add_user("ana@example.com", Some(PW), false);
    // A plain-name session, as a sibling subdomain could plant with Domain=example.com.
    let planted = h.login("ana@example.com", PW).await;
    assert!(planted.starts_with("cairn_session="));
    let res = Req::get("/api/me")
        .cookie(&planted)
        .peer("10.0.0.1:5000")
        .header("x-forwarded-proto", "https")
        .send(&h.app)
        .await;
    assert_eq!(res.json, json!({ "user": null }));
    let plain_http = Req::get("/api/me").cookie(&planted).send(&h.app).await;
    assert_eq!(plain_http.json["user"]["email"], "ana@example.com");
}
