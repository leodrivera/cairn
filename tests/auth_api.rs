//! Sign-in, sessions and profile, through the real router.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use cairn::auth::session;
use cairn::users;
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

#[derive(Clone, Default)]
struct LogBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn failed_login_logs_email_as_typed() {
    let buf = LogBuf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    Req::post("/api/auth/login")
        .json(login_body(" Ana@Example.COM ", "Wrong1!x"))
        .send(&h.app)
        .await;
    let log = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    let line = log
        .lines()
        .find(|l| l.contains("auth.login_failed"))
        .expect("auth.login_failed logged");
    assert!(line.contains(r#""email":" Ana@Example.COM ""#), "{line}");
}

#[tokio::test]
async fn login_ends_the_session_the_browser_had() {
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    h.add_user("bo@example.com", Some(PW), false);
    let old = h.login("ana@example.com", PW).await;
    let res = Req::post("/api/auth/login")
        .cookie(&old)
        .json(login_body("bo@example.com", PW))
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(
        Req::get("/api/me").cookie(&old).send(&h.app).await.json,
        json!({ "user": null })
    );
}

#[tokio::test]
async fn dev_user_is_every_request() {
    let h = harness(&[("CAIRN_DEV_USER", "Admin@Example.com")]);
    h.add_user("admin@example.com", Some(PW), true);
    let res = Req::get("/api/me").send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["user"]["email"], json!("admin@example.com"));
    let res = Req::get("/api/files").send(&h.app).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
}

#[tokio::test]
async fn dev_user_unknown_is_signed_out() {
    let h = harness(&[("CAIRN_DEV_USER", "nobody@example.com")]);
    h.add_user("admin@example.com", Some(PW), true);
    let res = Req::get("/api/me").send(&h.app).await;
    assert_eq!(res.json["user"], json!(null));
}

#[tokio::test]
async fn parallel_logins_cannot_pass_the_limit() {
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    let tries = (0..15).map(|_| {
        Req::post("/api/auth/login")
            .json(login_body("ana@example.com", "Wrong1!x"))
            .peer("203.0.113.9:4000")
            .send(&h.app)
    });
    let results = futures_util::future::join_all(tries).await;
    let wrong = results
        .iter()
        .filter(|r| r.status == StatusCode::UNAUTHORIZED)
        .count();
    let limited = results
        .iter()
        .filter(|r| r.status == StatusCode::TOO_MANY_REQUESTS)
        .count();
    assert_eq!((wrong, limited), (5, 10));
}

#[tokio::test]
async fn right_passwords_do_not_use_up_the_address() {
    // A household behind one address signs in often; only failures count against it.
    let h = harness(&[]);
    for i in 0..12 {
        h.add_user(&format!("u{i}@example.com"), Some(PW), false);
    }
    for i in 0..12 {
        let res = Req::post("/api/auth/login")
            .json(login_body(&format!("u{i}@example.com"), PW))
            .peer("203.0.113.10:4000")
            .send(&h.app)
            .await;
        assert_eq!(res.status, StatusCode::OK, "sign-in {i}");
    }
    let res = Req::post("/api/auth/login")
        .json(login_body("u0@example.com", "Wrong1!x"))
        .peer("203.0.113.10:4000")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn parallel_password_changes_cannot_pass_the_limit() {
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    let a = h.login("ana@example.com", PW).await;
    let tries = (0..8).map(|_| {
        Req::put("/api/me/password")
            .cookie(&a)
            .json(json!({ "current": "Nope123!", "next": "Ghijkl2?" }))
            .send(&h.app)
    });
    let results = futures_util::future::join_all(tries).await;
    let wrong = results
        .iter()
        .filter(|r| r.status == StatusCode::BAD_REQUEST)
        .count();
    let limited = results
        .iter()
        .filter(|r| r.status == StatusCode::TOO_MANY_REQUESTS)
        .count();
    assert_eq!((wrong, limited), (5, 3));
}

#[tokio::test]
async fn right_current_password_is_not_counted() {
    // A right current password with a weak new one is a slip in the new one, not a guess.
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    let a = h.login("ana@example.com", PW).await;
    let put = |current: &str| {
        Req::put("/api/me/password")
            .cookie(&a)
            .json(json!({ "current": current, "next": "weak" }))
    };
    for _ in 0..4 {
        put("Nope123!").send(&h.app).await;
    }
    for _ in 0..3 {
        assert_eq!(put(PW).send(&h.app).await.code(), "weak_password");
    }
    assert_eq!(put("Nope123!").send(&h.app).await.code(), "wrong_password");
    assert_eq!(
        put("Nope123!").send(&h.app).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

/// What the owner or an admin does while a password check is on its way: the slow hash gives it time to land.
async fn sign_in_during(h: &common::Harness, email: &str, write: impl FnOnce()) -> common::Res {
    let login = Req::post("/api/auth/login")
        .json(json!({ "email": email, "password": PW }))
        .send(&h.app);
    let change = async {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        write();
    };
    tokio::join!(login, change).0
}

async fn alive(h: &common::Harness, res: &common::Res) -> bool {
    let Some(cookie) = res.cookie() else {
        return false;
    };
    let me = Req::get("/api/me").cookie(&cookie).send(&h.app).await;
    me.json["user"].is_object()
}

#[tokio::test]
async fn sign_in_checked_against_an_old_password_gets_no_session() {
    let h = harness(&[]);
    let user = h.add_user("ana@example.com", Some(PW), false);
    let res = sign_in_during(&h, "ana@example.com", || {
        users::set_password(&h.state.db, user.id, Some("new hash".into())).unwrap();
        session::delete_for_user(&h.state.db, user.id, None).unwrap();
    })
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert!(!alive(&h, &res).await);
}

#[tokio::test]
async fn sign_in_checked_against_a_removed_password_gets_no_session() {
    let h = harness(&[]);
    let user = h.add_user("ana@example.com", Some(PW), false);
    let res = sign_in_during(&h, "ana@example.com", || {
        users::set_password(&h.state.db, user.id, None).unwrap();
        session::delete_for_user(&h.state.db, user.id, None).unwrap();
    })
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert!(!alive(&h, &res).await);
}

#[tokio::test]
async fn sign_in_checked_before_an_address_change_gets_no_session() {
    let h = harness(&[]);
    let user = h.add_user("ana@example.com", Some(PW), false);
    let res = sign_in_during(&h, "ana@example.com", || {
        let patch = users::UserPatch {
            email: Some("bo@example.com".into()),
            ..users::UserPatch::default()
        };
        users::update(&h.state.db, user.id, patch).unwrap();
        session::delete_for_user(&h.state.db, user.id, None).unwrap();
    })
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

/// `write` lands while the change is hashing; the password it set must stay.
async fn change_during(h: &common::Harness, cookie: &str, write: impl FnOnce()) -> common::Res {
    let change = Req::put("/api/me/password")
        .cookie(cookie)
        .json(json!({ "current": PW, "next": "Newpass1!" }))
        .send(&h.app);
    let reset = async {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        write();
    };
    tokio::join!(change, reset).0
}

fn stored_hash(h: &common::Harness, email: &str) -> Option<String> {
    users::by_email(&h.state.db, email).unwrap().unwrap().1
}

#[tokio::test]
async fn password_change_yields_to_a_reset_that_landed_first() {
    let h = harness(&[]);
    let user = h.add_user("ana@example.com", Some(PW), false);
    let c = h.login("ana@example.com", PW).await;
    let res = change_during(&h, &c, || {
        users::set_password(&h.state.db, user.id, Some("admin hash".into())).unwrap();
    })
    .await;
    assert_eq!(
        (res.status, res.code()),
        (StatusCode::CONFLICT, "password_changed"),
        "{}",
        res.json
    );
    assert_eq!(
        stored_hash(&h, "ana@example.com").as_deref(),
        Some("admin hash")
    );
}

#[tokio::test]
async fn password_change_yields_to_a_removal_that_landed_first() {
    let h = harness(&[]);
    let user = h.add_user("ana@example.com", Some(PW), false);
    let c = h.login("ana@example.com", PW).await;
    let res = change_during(&h, &c, || {
        users::set_password(&h.state.db, user.id, None).unwrap();
    })
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.json);
    assert_eq!(stored_hash(&h, "ana@example.com"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn refused_captcha_answers_are_limited_per_address() {
    let url = fake_vendor(r#"{"success":false}"#).await;
    let h = harness(CAPTCHA_ON).with_captcha_url(url);
    let attempt = |n: u32| {
        let body = json!({
            "email": format!("user{n}@example.com"), "password": PW, "keep": false, "captchaToken": "tok"
        });
        Req::post("/api/auth/login").json(body).send(&h.app)
    };
    for n in 0..30 {
        assert_eq!(attempt(n).await.status, StatusCode::BAD_REQUEST);
    }
    assert_eq!(attempt(30).await.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test(flavor = "multi_thread")]
async fn accepted_captcha_answers_do_not_count() {
    let url = fake_vendor(r#"{"success":true,"hostname":"files.example.com"}"#).await;
    let h = harness(CAPTCHA_ON).with_captcha_url(url);
    h.add_user("ana@example.com", Some(PW), false);
    for _ in 0..31 {
        let body = json!({
            "email": "ana@example.com", "password": PW, "keep": false, "captchaToken": "tok"
        });
        let res = Req::post("/api/auth/login").json(body).send(&h.app).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    }
}
