//! Sign-in through the OAuth broker: provider buttons, `/auth/oauth/{provider}` and `/auth/sso`.

mod common;

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use common::broker::FakeBroker;
use common::{Harness, Req, harness};
use serde_json::json;

const SECRET: &str = "0123456789abcdef0123456789abcdef";

fn google() -> serde_json::Value {
    json!([{ "id": "google", "name": "Google", "oauth": true }])
}

fn oauth(broker: &FakeBroker, extra: &[(&str, &str)]) -> Harness {
    let mut vars = vec![
        ("CAIRN_AUTH_METHODS", "password,oauth"),
        ("CAIRN_PUBLIC_URL", "https://files.example.com"),
        ("CAIRN_OAUTH_BROKER_URL", broker.url.as_str()),
        ("CAIRN_OAUTH_SECRET", SECRET),
    ];
    vars.extend_from_slice(extra);
    harness(&vars)
}

async fn ready(broker: &FakeBroker, extra: &[(&str, &str)]) -> Harness {
    let h = oauth(broker, extra);
    h.broker_ready().await;
    h
}

#[tokio::test]
async fn config_lists_broker_providers() {
    let b = FakeBroker::start(google()).await;
    let h = ready(&b, &[]).await;
    let res = Req::get("/api/config").send(&h.app).await;
    assert_eq!(
        res.json["providers"],
        json!([{ "name": "Google", "href": "/auth/oauth/google" }])
    );
}

#[tokio::test]
async fn config_without_oauth_has_none() {
    let h = harness(&[]);
    assert_eq!(
        Req::get("/api/config").send(&h.app).await.json["providers"],
        json!([])
    );
}

#[tokio::test]
async fn config_does_not_wait_on_broker() {
    let b = FakeBroker::start(google()).await;
    b.set_delay(Duration::from_secs(3));
    let h = oauth(&b, &[]);
    let t = Instant::now();
    let res = Req::get("/api/config").send(&h.app).await;
    assert!(
        t.elapsed() < Duration::from_millis(200),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(res.json["providers"], json!([]));
}

#[tokio::test]
async fn start_redirects_and_relays_cookies() {
    let b = FakeBroker::start(google()).await;
    let h = ready(&b, &[]).await;
    let res = Req::get("/auth/oauth/google").send(&h.app).await;
    assert_eq!(res.status, StatusCode::FOUND);
    assert_eq!(
        res.header("location"),
        "https://provider.example.com/authorize?state=abc"
    );
    let cookies: Vec<_> = res
        .headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    assert_eq!(cookies.len(), 2, "{cookies:?}");
    assert!(cookies[0].starts_with("tinyauth-csrf=one"));
    assert!(cookies[1].starts_with("tinyauth-redirect=two"));
    assert_eq!(
        b.last_redirect_uri().as_deref(),
        Some("https://files.example.com/auth/sso")
    );
}

#[tokio::test]
async fn start_unknown_provider_goes_to_login() {
    let b = FakeBroker::start(google()).await;
    let h = ready(&b, &[]).await;
    let res = Req::get("/auth/oauth/github").send(&h.app).await;
    assert_eq!(res.status, StatusCode::FOUND);
    assert_eq!(res.header("location"), "/login?error=sso");
    assert_eq!(b.last_redirect_uri(), None, "the broker was not asked");
}

#[tokio::test]
async fn start_off_is_404() {
    let h = harness(&[]);
    let res = Req::get("/auth/oauth/google").send(&h.app).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert!(res.header("content-type").starts_with("text/plain"));
}

#[tokio::test]
async fn start_broker_error_goes_to_login() {
    let b = FakeBroker::start(google()).await;
    let h = ready(&b, &[]).await;
    b.set_start_status(500);
    let res = Req::get("/auth/oauth/google").send(&h.app).await;
    assert_eq!(res.status, StatusCode::FOUND);
    assert_eq!(res.header("location"), "/login?error=sso");
}

#[tokio::test]
async fn start_http_authorize_refused() {
    let b = FakeBroker::start(google()).await;
    let h = ready(&b, &[]).await;
    b.set_start_url("http://provider.example.com/authorize");
    let res = Req::get("/auth/oauth/google").send(&h.app).await;
    assert_eq!(res.header("location"), "/login?error=sso");
    assert!(res.headers.get("set-cookie").is_none());
}

#[tokio::test]
async fn start_rate_limited() {
    let b = FakeBroker::start(google()).await;
    let h = ready(&b, &[]).await;
    for _ in 0..20 {
        let res = Req::get("/auth/oauth/google")
            .peer("203.0.113.9:4000")
            .send(&h.app)
            .await;
        assert_eq!(res.status, StatusCode::FOUND);
    }
    let res = Req::get("/auth/oauth/google")
        .peer("203.0.113.9:4000")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(!res.header("retry-after").is_empty());
    assert!(res.header("content-type").starts_with("text/plain"));
    assert!(
        String::from_utf8_lossy(&res.body).starts_with("Too many tries in a row. Wait 10 minutes")
    );
    let res = Req::get("/auth/oauth/google")
        .peer("198.51.100.7:4000")
        .send(&h.app)
        .await;
    assert_eq!(res.status, StatusCode::FOUND);
}

#[tokio::test]
async fn broker_url_is_used() {
    let b = FakeBroker::start(google()).await;
    let h = ready(&b, &[]).await;
    let before = b.hits();
    assert!(before >= 1);
    Req::get("/auth/oauth/google").send(&h.app).await;
    assert_eq!(b.hits(), before + 1);
}

/// Every `name=value` the response set.
fn cookies(res: &common::Res) -> Vec<String> {
    res.headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect()
}

fn cookie_named(res: &common::Res, name: &str) -> Option<String> {
    cookies(res)
        .into_iter()
        .find(|c| c.starts_with(&format!("{name}=")))
}

fn sso(email: &str, secret: Option<&str>) -> Req {
    let mut r = Req::get("/auth/sso").header("remote-email", email);
    if let Some(s) = secret {
        r = r.header("x-auth-secret", s);
    }
    r
}

const PW: &str = "Correct-Horse-1";

async fn me(h: &Harness, cookie: &str) -> serde_json::Value {
    Req::get("/api/me").cookie(cookie).send(&h.app).await.json
}

#[tokio::test]
async fn sso_signs_in() {
    let b = FakeBroker::start(google()).await;
    let h = oauth(&b, &[]);
    h.add_user("ana@example.com", None, false);
    let res = sso("Ana@Example.COM", Some(SECRET)).send(&h.app).await;
    assert_eq!(res.status, StatusCode::FOUND);
    assert_eq!(res.header("location"), "/");
    let set = cookie_named(&res, "cairn_session").expect("session cookie");
    assert!(!set.contains("Max-Age"), "{set}");
    let cookie = set.split(';').next().unwrap().to_string();
    let body = me(&h, &cookie).await;
    assert_eq!(body["user"]["email"], json!("ana@example.com"));
    assert_eq!(body["method"], json!("oauth"));
}

#[tokio::test]
async fn sso_wrong_secret() {
    let b = FakeBroker::start(google()).await;
    let h = oauth(&b, &[]);
    h.add_user("ana@example.com", None, false);
    let wrong = format!("{}X", &SECRET[..SECRET.len() - 1]);
    let res = sso("ana@example.com", Some(&wrong)).send(&h.app).await;
    assert_eq!(res.header("location"), "/login?error=sso");
    let set = cookie_named(&res, "cairn_session").unwrap_or_default();
    assert!(set.is_empty() || set.contains("Max-Age=0"), "{set}");
}

#[tokio::test]
async fn sso_missing_secret() {
    let b = FakeBroker::start(google()).await;
    let h = oauth(&b, &[]);
    h.add_user("ana@example.com", None, false);
    let res = sso("ana@example.com", None).send(&h.app).await;
    assert_eq!(res.header("location"), "/login?error=sso");
}

#[tokio::test]
async fn sso_secret_from_file() {
    let b = FakeBroker::start(google()).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oauth_secret");
    std::fs::write(&path, format!("{SECRET}\n")).unwrap();
    let path = path.to_str().unwrap().to_string();
    let h = harness(&[
        ("CAIRN_AUTH_METHODS", "password,oauth"),
        ("CAIRN_PUBLIC_URL", "https://files.example.com"),
        ("CAIRN_OAUTH_BROKER_URL", b.url.as_str()),
        ("CAIRN_OAUTH_SECRET_FILE", path.as_str()),
    ]);
    h.add_user("ana@example.com", None, false);
    // The file's value, newline dropped, is the secret; the inline default of other tests is not in play.
    let res = sso("ana@example.com", Some(&format!("{SECRET}x")))
        .send(&h.app)
        .await;
    assert_eq!(res.header("location"), "/login?error=sso");
    let res = sso("ana@example.com", Some(SECRET)).send(&h.app).await;
    assert_eq!(res.header("location"), "/");
}

#[tokio::test]
async fn sso_planted_session_ends() {
    let b = FakeBroker::start(google()).await;
    let h = oauth(&b, &[]);
    h.add_user("ana@example.com", None, false);
    h.add_user("bo@example.com", Some(PW), false);
    let bo = h.login("bo@example.com", PW).await;
    let res = sso("ana@example.com", Some("wrong"))
        .cookie(&bo)
        .send(&h.app)
        .await;
    assert_eq!(res.header("location"), "/login?error=sso");
    assert_eq!(me(&h, &bo).await["user"], json!(null));

    let bo = h.login("bo@example.com", PW).await;
    let res = sso("ana@example.com", Some(SECRET))
        .cookie(&bo)
        .send(&h.app)
        .await;
    assert_eq!(me(&h, &bo).await["user"], json!(null));
    let ana = res.cookie().unwrap();
    assert_eq!(
        me(&h, &ana).await["user"]["email"],
        json!("ana@example.com")
    );
}

#[tokio::test]
async fn sso_unknown_email() {
    let b = FakeBroker::start(google()).await;
    let h = oauth(&b, &[]);
    let res = sso("nobody@example.com", Some(SECRET)).send(&h.app).await;
    assert_eq!(res.header("location"), "/login?error=no-account");
    let denied = cookie_named(&res, "cairn_denied").expect("denied cookie");
    assert!(
        denied.contains("HttpOnly") && denied.contains("Max-Age=300"),
        "{denied}"
    );
    assert!(!denied.contains("nobody@example.com"), "{denied}");
    let pair = denied.split(';').next().unwrap().to_string();
    let res = Req::get("/api/me").cookie(&pair).send(&h.app).await;
    assert_eq!(
        res.json,
        json!({ "user": null, "deniedEmail": "nobody@example.com" })
    );
    let cleared = cookie_named(&res, "cairn_denied").expect("clearing cookie");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    assert_eq!(
        Req::get("/api/me").send(&h.app).await.json,
        json!({ "user": null })
    );
}

#[tokio::test]
async fn sso_never_creates_users() {
    let b = FakeBroker::start(google()).await;
    let h = oauth(&b, &[]);
    h.add_user("admin@example.com", Some(PW), true);
    sso("nobody@example.com", Some(SECRET)).send(&h.app).await;
    assert_eq!(cairn::users::count(&h.state.db).unwrap(), 1);
}

#[tokio::test]
async fn sso_invalid_email() {
    let b = FakeBroker::start(google()).await;
    let h = oauth(&b, &[]);
    let res = sso("not-an-email", Some(SECRET)).send(&h.app).await;
    assert_eq!(res.header("location"), "/login?error=sso");
}

#[tokio::test]
async fn sso_off_is_404() {
    let h = harness(&[]);
    h.add_user("ana@example.com", Some(PW), false);
    let res = sso("ana@example.com", Some(SECRET)).send(&h.app).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert!(res.headers.get("set-cookie").is_none());
}

#[tokio::test]
async fn sso_path_variants_404() {
    let b = FakeBroker::start(google()).await;
    let h = oauth(&b, &[]);
    h.add_user("ana@example.com", None, false);
    for path in [
        "/auth/sso/",
        "/AUTH/SSO",
        "/auth//sso",
        "/Auth/Sso",
        "/auth",
        "/auth/",
    ] {
        let res = Req::get(path)
            .header("remote-email", "ana@example.com")
            .header("x-auth-secret", SECRET)
            .send(&h.app)
            .await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
        assert!(
            res.header("content-type").starts_with("text/plain"),
            "{path}"
        );
        assert!(res.headers.get("set-cookie").is_none(), "{path}");
    }
}

#[tokio::test]
async fn remote_email_ignored_elsewhere() {
    let b = FakeBroker::start(google()).await;
    let h = oauth(&b, &[]);
    h.add_user("ana@example.com", None, false);
    for path in ["/api/me", "/"] {
        let res = Req::get(path)
            .header("remote-email", "ana@example.com")
            .header("x-auth-secret", SECRET)
            .send(&h.app)
            .await;
        assert!(res.headers.get("set-cookie").is_none(), "{path}");
        if path == "/api/me" {
            assert_eq!(res.json, json!({ "user": null }));
        }
    }
}

#[tokio::test]
async fn logout_after_oauth_goes_to_broker() {
    let b = FakeBroker::start(google()).await;
    let h = oauth(
        &b,
        &[("CAIRN_OAUTH_LOGOUT_URL", "https://auth.example.com/logout")],
    );
    h.add_user("ana@example.com", Some(PW), false);
    let c = sso("ana@example.com", Some(SECRET))
        .send(&h.app)
        .await
        .cookie()
        .unwrap();
    let res = Req::post("/api/auth/logout").cookie(&c).send(&h.app).await;
    assert_eq!(
        res.json,
        json!({ "next": "https://auth.example.com/logout" })
    );
    let c = h.login("ana@example.com", PW).await;
    let res = Req::post("/api/auth/logout").cookie(&c).send(&h.app).await;
    assert_eq!(res.json, json!({ "next": null }));
}

#[tokio::test]
async fn logout_without_logout_url_is_null() {
    let b = FakeBroker::start(google()).await;
    let h = oauth(&b, &[]);
    h.add_user("ana@example.com", None, false);
    let c = sso("ana@example.com", Some(SECRET))
        .send(&h.app)
        .await
        .cookie()
        .unwrap();
    let res = Req::post("/api/auth/logout").cookie(&c).send(&h.app).await;
    assert_eq!(res.json, json!({ "next": null }));
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
async fn unknown_email_log_has_name() {
    let buf = LogBuf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let b = FakeBroker::start(google()).await;
    let h = oauth(&b, &[]);
    sso("nobody@example.com", Some(SECRET))
        .header("remote-name", "Ana")
        .send(&h.app)
        .await;
    let log = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    let line = log
        .lines()
        .find(|l| l.contains("auth.unknown_email"))
        .expect("auth.unknown_email logged");
    assert!(line.contains(r#""name":"Ana""#), "{line}");
    assert!(line.contains(r#""email":"nobody@example.com""#), "{line}");
    assert!(!log.contains(SECRET));
}

#[tokio::test]
async fn href_encodes_provider_id() {
    let b =
        FakeBroker::start(json!([{ "id": "my provider/x", "name": "Odd", "oauth": true }])).await;
    let h = ready(&b, &[]).await;
    let res = Req::get("/api/config").send(&h.app).await;
    let href = res.json["providers"][0]["href"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(href, "/auth/oauth/my%20provider%2Fx");
    let res = Req::get(&href).send(&h.app).await;
    assert_eq!(res.status, StatusCode::FOUND);
    assert!(
        res.header("location")
            .starts_with("https://provider.example.com/")
    );
}

#[tokio::test]
async fn start_before_first_list_goes_to_login() {
    let b = FakeBroker::start(google()).await;
    b.set_delay(Duration::from_secs(3));
    let h = oauth(&b, &[]);
    let res = Req::get("/auth/oauth/google").send(&h.app).await;
    assert_eq!(res.status, StatusCode::FOUND);
    assert_eq!(res.header("location"), "/login?error=sso");
}

#[tokio::test]
async fn session_failure_clears_cookie() {
    let b = FakeBroker::start(google()).await;
    let h = oauth(&b, &[]);
    h.add_user("ana@example.com", None, false);
    h.state
        .db
        .with(|c| c.execute_batch("DROP TABLE sessions"))
        .unwrap();
    let res = sso("ana@example.com", Some(SECRET)).send(&h.app).await;
    assert_eq!(res.header("location"), "/login?error=sso");
    let c = cookie_named(&res, "cairn_session").expect("clearing cookie");
    assert!(c.contains("Max-Age=0"), "{c}");
}

#[tokio::test]
async fn config_serves_stale_list_while_broker_hangs() {
    let b = FakeBroker::start(google()).await;
    let h = ready(&b, &[]).await;
    b.set_delay(Duration::from_secs(3));
    h.state.broker.as_ref().unwrap().mark_stale();
    let t = Instant::now();
    let res = Req::get("/api/config").send(&h.app).await;
    assert!(
        t.elapsed() < Duration::from_millis(200),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(
        res.json["providers"],
        json!([{ "name": "Google", "href": "/auth/oauth/google" }])
    );
}

#[tokio::test]
async fn other_methods_on_auth_are_404() {
    let b = FakeBroker::start(google()).await;
    let h = ready(&b, &[]).await;
    for path in [
        "/auth/sso",
        "/auth/oauth/google",
        "/auth/oidc/fake",
        "/auth/oidc/fake/callback",
    ] {
        for req in [Req::post(path), Req::put(path), Req::delete(path)] {
            let res = req.send(&h.app).await;
            assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
            assert!(
                res.header("content-type").starts_with("text/plain"),
                "{path}"
            );
        }
    }
}

#[tokio::test]
async fn several_remote_emails_are_refused() {
    let b = FakeBroker::start(google()).await;
    let h = oauth(&b, &[]);
    h.add_user("ana@example.com", None, false);
    h.add_user("admin@example.com", None, true);
    let res = sso("admin@example.com", Some(SECRET))
        .header("remote-email", "ana@example.com")
        .send(&h.app)
        .await;
    assert_eq!(res.header("location"), "/login?error=sso");
    let set = cookie_named(&res, "cairn_session").unwrap_or_default();
    assert!(set.is_empty() || set.contains("Max-Age=0"), "{set}");
}
