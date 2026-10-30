//! Sign-in with OpenID Connect, run by Cairn: `/auth/oidc/{id}` and its callback, against a fake provider.

mod common;

use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use common::broker::FakeBroker;
use common::idp::{CLIENT_SECRET, FakeIdp};
use common::{Harness, Req, Res, harness};
use serde_json::json;
use sha2::{Digest, Sha256};

const ISSUER: &str = "https://idp.example.com";
const PW: &str = "Correct-Horse-1";

fn vars<'a>(extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut v = vec![
        ("CAIRN_AUTH_METHODS", "password,oidc"),
        ("CAIRN_PUBLIC_URL", "https://files.example.com"),
        ("CAIRN_OIDC_PROVIDERS", "fake"),
        ("CAIRN_OIDC_FAKE_ISSUER", ISSUER),
        ("CAIRN_OIDC_FAKE_CLIENT_ID", "cairn-client"),
        ("CAIRN_OIDC_FAKE_CLIENT_SECRET", CLIENT_SECRET),
        ("CAIRN_OIDC_FAKE_NAME", "Example ID"),
    ];
    for (k, val) in extra {
        v.retain(|(name, _)| name != k);
        v.push((k, val));
    }
    v
}

async fn ready(idp: &FakeIdp, extra: &[(&str, &str)]) -> Harness {
    let h = harness(&vars(extra));
    h.oidc_ready(idp).await;
    h
}

fn set_cookies(res: &Res) -> Vec<String> {
    res.headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect()
}

fn cookie_named(res: &Res, name: &str) -> Option<String> {
    set_cookies(res)
        .into_iter()
        .find(|c| c.starts_with(&format!("{name}=")))
}

fn param(location: &str, name: &str) -> String {
    url::Url::parse(location)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

/// Starts a sign-in: the authorize URL and the pending cookie (`name=value`).
async fn start(h: &Harness, idp: &FakeIdp) -> (String, String) {
    let res = Req::get("/auth/oidc/fake").send(&h.app).await;
    assert_eq!(
        res.status,
        StatusCode::FOUND,
        "{:?}",
        String::from_utf8_lossy(&res.body)
    );
    let location = res.header("location").to_string();
    idp.authorize(&location);
    let cookie = cookie_named(&res, "cairn_oidc").expect("pending cookie");
    (location, cookie.split(';').next().unwrap().to_string())
}

async fn callback(h: &Harness, state: &str, cookie: &str) -> Res {
    Req::get(&format!("/auth/oidc/fake/callback?code=c1&state={state}"))
        .cookie(cookie)
        .send(&h.app)
        .await
}

/// A whole sign-in; the callback's response.
async fn sign_in(h: &Harness, idp: &FakeIdp) -> Res {
    let (location, cookie) = start(h, idp).await;
    callback(h, &param(&location, "state"), &cookie).await
}

async fn me(h: &Harness, cookie: &str) -> serde_json::Value {
    Req::get("/api/me").cookie(cookie).send(&h.app).await.json
}

fn session_of(res: &Res) -> String {
    cookie_named(res, "cairn_session")
        .expect("session cookie")
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn config_lists_ready_provider() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    let res = Req::get("/api/config").send(&h.app).await;
    assert_eq!(
        res.json["providers"],
        json!([{ "name": "Example ID", "href": "/auth/oidc/fake" }])
    );
    let h = ready(&idp, &[("CAIRN_OIDC_FAKE_NAME", "")]).await;
    let res = Req::get("/api/config").send(&h.app).await;
    assert_eq!(res.json["providers"][0]["name"], json!("fake"));
}

#[tokio::test]
async fn config_hides_unready_provider() {
    let idp = FakeIdp::start(ISSUER).await;
    idp.set_down(true);
    let h = ready(&idp, &[]).await;
    let res = Req::get("/api/config").send(&h.app).await;
    assert_eq!(res.json["providers"], json!([]));
}

#[tokio::test]
async fn start_redirects_with_pkce() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    let res = Req::get("/auth/oidc/fake").send(&h.app).await;
    assert_eq!(res.status, StatusCode::FOUND);
    let loc = res.header("location").to_string();
    assert!(
        loc.starts_with(&format!("{}/authorize?", idp.base)),
        "{loc}"
    );
    assert_eq!(param(&loc, "response_type"), "code");
    assert_eq!(param(&loc, "client_id"), "cairn-client");
    assert_eq!(
        param(&loc, "redirect_uri"),
        "https://files.example.com/auth/oidc/fake/callback"
    );
    assert_eq!(param(&loc, "scope"), "openid email profile");
    assert_eq!(param(&loc, "code_challenge_method"), "S256");
    for p in ["state", "nonce", "code_challenge"] {
        assert_eq!(param(&loc, p).len(), 43, "{p}");
    }
    let c = cookie_named(&res, "cairn_oidc").unwrap();
    for part in [
        "Path=/auth/oidc/",
        "HttpOnly",
        "Max-Age=600",
        "SameSite=Lax",
    ] {
        assert!(c.contains(part), "{c}");
    }
}

#[tokio::test]
async fn callback_signs_in() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    h.add_user("ana@example.com", None, false);
    idp.set_claims(json!({ "email": "Ana@Example.COM" }));
    let res = sign_in(&h, &idp).await;
    assert_eq!(res.header("location"), "/");
    let c = session_of(&res);
    let body = me(&h, &c).await;
    assert_eq!(body["user"]["email"], json!("ana@example.com"));
    assert_eq!(body["method"], json!("oidc"));
    let cleared = cookie_named(&res, "cairn_oidc").unwrap();
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
}

#[tokio::test]
async fn es256_signs_in() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    h.add_user("ana@example.com", None, false);
    idp.set_alg("ES256");
    assert_eq!(sign_in(&h, &idp).await.header("location"), "/");
}

fn form(body: &str) -> Vec<(String, String)> {
    form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect()
}

fn field(body: &str, name: &str) -> Option<String> {
    form(body)
        .into_iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v)
}

#[tokio::test]
async fn token_request_carries_secret_and_verifier() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    h.add_user("ana@example.com", None, false);
    let (loc, cookie) = start(&h, &idp).await;
    callback(&h, &param(&loc, "state"), &cookie).await;
    let (headers, body) = idp.token_requests().pop().unwrap();
    assert!(headers.get("authorization").is_none());
    assert_eq!(
        field(&body, "grant_type").as_deref(),
        Some("authorization_code")
    );
    assert_eq!(field(&body, "code").as_deref(), Some("c1"));
    assert_eq!(field(&body, "client_id").as_deref(), Some("cairn-client"));
    assert_eq!(
        field(&body, "client_secret").as_deref(),
        Some(CLIENT_SECRET)
    );
    assert_eq!(
        field(&body, "redirect_uri").as_deref(),
        Some("https://files.example.com/auth/oidc/fake/callback")
    );
    let verifier = field(&body, "code_verifier").unwrap();
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        param(&loc, "code_challenge")
    );
}

#[tokio::test]
async fn basic_auth_when_only_basic_listed() {
    let idp = FakeIdp::start(ISSUER).await;
    idp.set_auth_methods(Some(vec!["client_secret_basic"]));
    let h = ready(&idp, &[]).await;
    h.add_user("ana@example.com", None, false);
    assert_eq!(sign_in(&h, &idp).await.header("location"), "/");
    let (headers, body) = idp.token_requests().pop().unwrap();
    let want = format!(
        "Basic {}",
        STANDARD.encode(format!("cairn-client:{CLIENT_SECRET}"))
    );
    assert_eq!(
        headers.get("authorization").unwrap().to_str().unwrap(),
        want
    );
    assert!(field(&body, "client_secret").is_none());
}

#[tokio::test]
async fn post_when_both_listed() {
    let idp = FakeIdp::start(ISSUER).await;
    idp.set_auth_methods(Some(vec!["client_secret_basic", "client_secret_post"]));
    let h = ready(&idp, &[]).await;
    h.add_user("ana@example.com", None, false);
    sign_in(&h, &idp).await;
    let (headers, body) = idp.token_requests().pop().unwrap();
    assert!(headers.get("authorization").is_none());
    assert_eq!(
        field(&body, "client_secret").as_deref(),
        Some(CLIENT_SECRET)
    );
}

#[tokio::test]
async fn client_secret_from_file() {
    let idp = FakeIdp::start(ISSUER).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret");
    std::fs::write(&path, "secret-from-file\n").unwrap();
    let path = path.to_str().unwrap().to_string();
    let mut v = vars(&[]);
    v.retain(|(k, _)| *k != "CAIRN_OIDC_FAKE_CLIENT_SECRET");
    v.push(("CAIRN_OIDC_FAKE_CLIENT_SECRET_FILE", path.as_str()));
    let h = harness(&v);
    h.oidc_ready(&idp).await;
    h.add_user("ana@example.com", None, false);
    sign_in(&h, &idp).await;
    let (_, body) = idp.token_requests().pop().unwrap();
    assert_eq!(
        field(&body, "client_secret").as_deref(),
        Some("secret-from-file")
    );
}

fn no_session(res: &Res) {
    assert_eq!(res.header("location"), "/login?error=sso");
    let s = cookie_named(res, "cairn_session").unwrap_or_default();
    assert!(s.is_empty() || s.contains("Max-Age=0"), "{s}");
}

#[tokio::test]
async fn wrong_state() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    h.add_user("ana@example.com", None, false);
    let (_, cookie) = start(&h, &idp).await;
    let res = callback(&h, "not-the-state", &cookie).await;
    no_session(&res);
    assert!(
        cookie_named(&res, "cairn_oidc")
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(idp.hits("token"), 0);
}

#[tokio::test]
async fn callback_without_cookie() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    let (loc, _) = start(&h, &idp).await;
    let res = Req::get(&format!(
        "/auth/oidc/fake/callback?code=c1&state={}",
        param(&loc, "state")
    ))
    .send(&h.app)
    .await;
    no_session(&res);
}

#[tokio::test]
async fn callback_twice() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    h.add_user("ana@example.com", None, false);
    let (loc, cookie) = start(&h, &idp).await;
    let first = callback(&h, &param(&loc, "state"), &cookie).await;
    assert_eq!(first.header("location"), "/");
    let session = session_of(&first);
    let again = callback(&h, &param(&loc, "state"), &format!("{cookie}; {session}")).await;
    assert_eq!(again.header("location"), "/login?error=sso");
    assert_eq!(idp.hits("token"), 1);
    // A reload after signing in must not sign the person out.
    assert_eq!(
        me(&h, &session).await["user"]["email"],
        json!("ana@example.com")
    );
    let rows: i64 = h
        .state
        .db
        .with(|c| c.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0)))
        .unwrap();
    assert!(rows <= 1, "{rows}");
}

#[tokio::test]
async fn replayed_nonce() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    h.add_user("ana@example.com", None, false);
    let (old, _) = start(&h, &idp).await;
    let (loc, cookie) = start(&h, &idp).await;
    idp.authorize(&old);
    no_session(&callback(&h, &param(&loc, "state"), &cookie).await);
}

async fn refused_with(claims: serde_json::Value, alg: &str) {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    h.add_user("ana@example.com", None, false);
    idp.set_claims(claims);
    idp.set_alg(alg);
    no_session(&sign_in(&h, &idp).await);
}

#[tokio::test]
async fn alg_none() {
    refused_with(json!({}), "none").await;
}

#[tokio::test]
async fn hs256() {
    refused_with(json!({}), "HS256").await;
}

#[tokio::test]
async fn wrong_aud() {
    refused_with(json!({ "aud": "other-client" }), "RS256").await;
}

#[tokio::test]
async fn expired() {
    refused_with(json!({ "exp": 1_000_000_000 }), "RS256").await;
}

#[tokio::test]
async fn email_unverified() {
    refused_with(json!({ "email_verified": false }), "RS256").await;
}

#[tokio::test]
async fn key_rotation() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    h.add_user("ana@example.com", None, false);
    let (loc, cookie) = start(&h, &idp).await;
    idp.rotate_key();
    let res = callback(&h, &param(&loc, "state"), &cookie).await;
    assert_eq!(res.header("location"), "/");
}

#[tokio::test]
async fn unknown_email() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    idp.set_claims(json!({ "email": "nobody@example.com" }));
    let res = sign_in(&h, &idp).await;
    assert_eq!(res.header("location"), "/login?error=no-account");
    assert!(cookie_named(&res, "cairn_denied").is_some());
    assert_eq!(cairn::users::count(&h.state.db).unwrap(), 0);
}

#[tokio::test]
async fn planted_session_ends() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    h.add_user("ana@example.com", None, false);
    h.add_user("bo@example.com", Some(PW), false);
    let bo = h.login("bo@example.com", PW).await;
    let (loc, cookie) = start(&h, &idp).await;
    let res = callback(&h, &param(&loc, "state"), &format!("{cookie}; {bo}")).await;
    assert_eq!(me(&h, &bo).await["user"], json!(null));
    assert_eq!(
        me(&h, &session_of(&res)).await["user"]["email"],
        json!("ana@example.com")
    );
}

#[tokio::test]
async fn provider_error() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    let (loc, cookie) = start(&h, &idp).await;
    let res = Req::get(&format!(
        "/auth/oidc/fake/callback?error=access_denied&state={}",
        param(&loc, "state")
    ))
    .cookie(&cookie)
    .send(&h.app)
    .await;
    no_session(&res);
    assert_eq!(idp.hits("token"), 0);
}

#[tokio::test]
async fn start_not_ready_goes_to_login() {
    let idp = FakeIdp::start(ISSUER).await;
    idp.set_down(true);
    let h = ready(&idp, &[]).await;
    let res = Req::get("/auth/oidc/fake").send(&h.app).await;
    assert_eq!(res.header("location"), "/login?error=sso");
}

#[tokio::test]
async fn start_rate_limited_shared() {
    let idp = FakeIdp::start(ISSUER).await;
    let broker =
        FakeBroker::start(json!([{ "id": "google", "name": "Google", "oauth": true }])).await;
    let h = ready(
        &idp,
        &[
            ("CAIRN_AUTH_METHODS", "password,oauth,oidc"),
            ("CAIRN_OAUTH_BROKER_URL", broker.url.as_str()),
            ("CAIRN_OAUTH_SECRET", "0123456789abcdef0123456789abcdef"),
        ],
    )
    .await;
    h.broker_ready().await;
    let ip = "203.0.113.9:4000";
    for _ in 0..10 {
        let res = Req::get("/auth/oauth/google").peer(ip).send(&h.app).await;
        assert_eq!(res.status, StatusCode::FOUND);
        let res = Req::get("/auth/oidc/fake").peer(ip).send(&h.app).await;
        assert_eq!(res.status, StatusCode::FOUND);
    }
    for path in ["/auth/oidc/fake", "/auth/oauth/google"] {
        let res = Req::get(path).peer(ip).send(&h.app).await;
        assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{path}");
    }
}

#[tokio::test]
async fn oidc_off_is_404() {
    let h = harness(&[]);
    for path in ["/auth/oidc/fake", "/auth/oidc/fake/callback?code=c&state=s"] {
        let res = Req::get(path).send(&h.app).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
        assert!(res.header("content-type").starts_with("text/plain"));
    }
}

#[tokio::test]
async fn unknown_provider_404() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    let res = Req::get("/auth/oidc/other").send(&h.app).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn oidc_path_variants_404() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[]).await;
    for path in [
        "/auth/oidc/fake/",
        "/AUTH/OIDC/fake",
        "/auth/oidc/fake/callback/",
    ] {
        let res = Req::get(path).send(&h.app).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn logout_after_oidc_next_null() {
    let idp = FakeIdp::start(ISSUER).await;
    let broker = FakeBroker::start(json!([])).await;
    let h = ready(
        &idp,
        &[
            ("CAIRN_AUTH_METHODS", "password,oauth,oidc"),
            ("CAIRN_OAUTH_BROKER_URL", broker.url.as_str()),
            ("CAIRN_OAUTH_SECRET", "0123456789abcdef0123456789abcdef"),
            ("CAIRN_OAUTH_LOGOUT_URL", "https://auth.example.com/logout"),
        ],
    )
    .await;
    h.add_user("ana@example.com", None, false);
    let c = session_of(&sign_in(&h, &idp).await);
    let res = Req::post("/api/auth/logout").cookie(&c).send(&h.app).await;
    assert_eq!(res.json, json!({ "next": null }));
}

fn https(r: Req) -> Req {
    r.peer("10.0.0.1:5000")
        .header("x-forwarded-for", "203.0.113.9")
        .header("x-forwarded-proto", "https")
}

#[tokio::test]
async fn https_pending_cookie_needs_host_prefix() {
    let idp = FakeIdp::start(ISSUER).await;
    let h = ready(&idp, &[("CAIRN_TRUSTED_PROXIES", "10.0.0.0/8")]).await;
    h.add_user("ana@example.com", None, false);
    let res = https(Req::get("/auth/oidc/fake")).send(&h.app).await;
    let loc = res.header("location").to_string();
    idp.authorize(&loc);
    let set = cookie_named(&res, "__Host-cairn_oidc").expect("__Host- pending cookie");
    for part in ["Path=/;", "Secure", "HttpOnly"] {
        assert!(set.contains(part), "{set}");
    }
    let value = set
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("__Host-cairn_oidc=")
        .to_string();
    let url = format!(
        "/auth/oidc/fake/callback?code=c1&state={}",
        param(&loc, "state")
    );
    // The same sealed value under the plain name, as a sibling site could plant it, does not count.
    let planted = https(Req::get(&url))
        .cookie(&format!("cairn_oidc={value}"))
        .send(&h.app)
        .await;
    assert_eq!(planted.header("location"), "/login?error=sso");
    assert_eq!(idp.hits("token"), 0);
    let res = https(Req::get(&url))
        .cookie(&format!("__Host-cairn_oidc={value}"))
        .send(&h.app)
        .await;
    assert_eq!(res.header("location"), "/");
}
