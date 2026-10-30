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
async fn start_unknown_provider_404() {
    let b = FakeBroker::start(google()).await;
    let h = ready(&b, &[]).await;
    let res = Req::get("/auth/oauth/github").send(&h.app).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert!(res.header("content-type").starts_with("text/plain"));
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
