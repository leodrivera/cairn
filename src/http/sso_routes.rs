//! Sign-in with a provider: page navigations outside `/api`. `GET /auth/oauth/{provider}` starts a broker sign-in;
//! `GET /auth/sso`, the only route the proxy puts behind the broker's forward-auth, turns its word into a session.

use std::net::IpAddr;

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Router, middleware};
use serde_json::{Value, json};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::auth::oidc::flow::{self, Pending};
use crate::auth::session::{self, Method};
use crate::http::AppState;
use crate::http::client::Client;
use crate::http::current_user::{self, CurrentUser, MaybeUser, clear_cookie, set_cookie};
use crate::http::error::ApiError;
use crate::users;

/// Carries the e-mail of a refused account to the sign-in page, once.
pub const DENIED_COOKIE: &str = "cairn_denied";

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        // Other methods get the same plain 404 as any unknown /auth path.
        .route(
            "/auth/oauth/{provider}",
            get(oauth_start).fallback(page_not_found),
        )
        .route("/auth/sso", get(sso).fallback(page_not_found))
        .route("/auth/oidc/{id}", get(oidc_start).fallback(page_not_found))
        .route(
            "/auth/oidc/{id}/callback",
            get(oidc_callback).fallback(page_not_found),
        )
        .layer(middleware::from_fn_with_state(state, current_user::load))
}

/// A page route that does not exist. Plain text, never the app page.
pub(crate) fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "Not found.",
    )
        .into_response()
}

async fn page_not_found() -> Response {
    not_found()
}

/// Percent-encodes one path segment: everything but the unreserved characters.
fn segment(id: &str) -> String {
    id.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Broker and OIDC starts share one limit per IP; every start counts. Over it: the 429 page.
pub(crate) fn start_limited(s: &AppState, ip: IpAddr) -> Option<Response> {
    let now = crate::now();
    if let Some(wait) = s.limiters.sso_start.blocked(ip, "", now) {
        tracing::warn!(event = "http.rate_limited", route = "sso_start", ip = %ip);
        let e = ApiError::too_many(wait);
        return Some(
            (
                StatusCode::TOO_MANY_REQUESTS,
                [
                    (
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("text/plain; charset=utf-8"),
                    ),
                    (header::RETRY_AFTER, HeaderValue::from(wait)),
                ],
                e.message,
            )
                .into_response(),
        );
    }
    s.limiters.sso_start.failed(ip, "", now);
    None
}

/// `302 Found`, the redirect every sign-in step answers.
pub(crate) fn found(location: &str) -> Response {
    let mut res = StatusCode::FOUND.into_response();
    if let Ok(v) = HeaderValue::from_str(location) {
        res.headers_mut().insert(header::LOCATION, v);
    }
    res
}

pub(crate) fn sign_in_failed() -> Response {
    found("/login?error=sso")
}

/// One `{ name, href }` per sign-in button: broker providers first, in the broker's order.
pub fn providers(s: &AppState) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(b) = &s.broker {
        for p in b.providers() {
            out.push(json!({ "name": p.name, "href": format!("/auth/oauth/{}", segment(&p.id)) }));
        }
    }
    for (id, name) in s.oidc.ready() {
        out.push(json!({ "name": name, "href": format!("/auth/oidc/{id}") }));
    }
    out
}

/// `<origin of CAIRN_PUBLIC_URL><path>`; the config check makes sure the public URL exists with oauth or oidc.
pub(crate) fn public(s: &AppState, path: &str) -> String {
    let origin = s
        .config
        .public_url
        .as_ref()
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default();
    format!("{origin}{path}")
}

async fn oauth_start(
    State(s): State<AppState>,
    client: Client,
    Path(provider): Path<String>,
) -> Response {
    let Some(broker) = &s.broker else {
        return not_found();
    };
    // Not in the list: a button from before a restart, or a stale page. Back to the sign-in page, not a dead end.
    if !broker.providers().iter().any(|p| p.id == provider) {
        tracing::warn!(event = "auth.oauth_start_failed", provider = %provider, ip = %client.ip, error = "unknown provider");
        return sign_in_failed();
    }
    if let Some(res) = start_limited(&s, client.ip) {
        return res;
    }
    match broker.start(&provider, &public(&s, "/auth/sso")).await {
        Ok((target, cookies)) => {
            let mut res = found(target.as_str());
            for c in cookies {
                res.headers_mut().append(header::SET_COOKIE, c);
            }
            res
        }
        Err(error) => {
            tracing::warn!(event = "auth.oauth_start_failed", provider = %provider, ip = %client.ip, error = %error);
            sign_in_failed()
        }
    }
}

pub(crate) fn denied_cookie(email: &str, https: bool) -> HeaderValue {
    let secure = if https { "; Secure" } else { "" };
    let value = URL_SAFE_NO_PAD.encode(email.as_bytes());
    HeaderValue::from_str(&format!(
        "{DENIED_COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age=300{secure}"
    ))
    .expect("ASCII")
}

pub(crate) fn clear_denied(https: bool) -> HeaderValue {
    let secure = if https { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{DENIED_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}"
    ))
    .expect("ASCII")
}

/// The refused e-mail the cookie carries, if it holds a valid one.
pub(crate) fn read_denied(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get_all(header::COOKIE)
        .iter()
        .flat_map(|v| v.to_str().unwrap_or("").split(';'))
        .find_map(|part| match part.trim().split_once('=') {
            Some((n, v)) if n == DENIED_COOKIE && !v.is_empty() => Some(v.to_string()),
            _ => None,
        })?;
    let bytes = URL_SAFE_NO_PAD.decode(raw).ok()?;
    users::normalize_email(std::str::from_utf8(&bytes).ok()?)
}

/// A provider sign-in that did not end in a session: the browser's old session is gone too.
pub(crate) fn failed(s: &AppState, client: Client, previous: Option<CurrentUser>) -> Response {
    end_previous(s, previous);
    let mut res = sign_in_failed();
    res.headers_mut()
        .append(header::SET_COOKIE, clear_cookie(client.https));
    res
}

fn end_previous(s: &AppState, previous: Option<CurrentUser>) {
    if let Some(old) = previous.filter(|c| !c.token.is_empty())
        && let Err(e) = session::delete(&s.db, &old.token)
    {
        tracing::error!(event = "db.error", error = %e);
    }
}

/// The shared ending of a broker or OIDC sign-in: whatever session the browser brought ends here, then the e-mail
/// the provider vouched for is matched to a user. Nobody is created.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn finish(
    s: &AppState,
    client: Client,
    user_agent: &str,
    previous: Option<CurrentUser>,
    email: &str,
    name: Option<&str>,
    method: Method,
    provider: Option<&str>,
) -> Response {
    // A provider vouches for the address as it sent it, so nothing but ASCII case and spaces may change on the way to
    // an account. The broker's header is ASCII already.
    let exact = method != Method::Oidc || users::normalizes_plainly(email);
    let Some(email) = users::normalize_email(email).filter(|_| exact) else {
        match method {
            Method::Oidc => {
                tracing::warn!(event = "auth.oidc_failed", provider = provider.unwrap_or(""), ip = %client.ip, reason = "email_invalid")
            }
            _ => {
                tracing::warn!(event = "auth.sso_failed", ip = %client.ip, reason = "email_invalid")
            }
        }
        return failed(s, client, previous);
    };
    let matched = match users::by_email(&s.db, &email) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(event = "db.error", error = %e.message());
            return failed(s, client, previous);
        }
    };
    let Some((user, _)) = matched else {
        let name: Option<String> = name.map(|n| n.trim().chars().take(200).collect());
        tracing::warn!(event = "auth.unknown_email", email = %email, name = name.as_deref(), ip = %client.ip, method = method.as_str());
        let mut res = failed(s, client, previous);
        res.headers_mut().insert(
            header::LOCATION,
            HeaderValue::from_static("/login?error=no-account"),
        );
        res.headers_mut()
            .append(header::SET_COOKIE, denied_cookie(&email, client.https));
        return res;
    };
    end_previous(s, previous);
    let token = match session::create(
        &s.db,
        user.id,
        method,
        false,
        &session::label_from_user_agent(user_agent),
        crate::now(),
    ) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(event = "db.error", error = %e);
            return failed(s, client, None);
        }
    };
    match method {
        Method::Oidc => {
            tracing::info!(event = "auth.oidc", user = user.id, email = %user.email, ip = %client.ip, method = "oidc", provider = provider.unwrap_or(""))
        }
        _ => {
            tracing::info!(event = "auth.sso", user = user.id, email = %user.email, ip = %client.ip, method = "oauth")
        }
    }
    let mut res = found("/");
    res.headers_mut()
        .append(header::SET_COOKIE, set_cookie(&token, false, client.https));
    res
}

/// Reads `Remote-Email` and `Remote-Name` here only, and only with the right `X-Auth-Secret`.
async fn sso(
    State(s): State<AppState>,
    client: Client,
    headers: HeaderMap,
    MaybeUser(previous): MaybeUser,
) -> Response {
    let Some(broker) = &s.config.auth.oauth else {
        return not_found();
    };
    let sent = headers.get("x-auth-secret");
    if !sent.is_some_and(|v| broker.secret.matches(v.as_bytes())) {
        tracing::warn!(event = "auth.proxy_secret_mismatch", ip = %client.ip, secret_sent = sent.is_some());
        return failed(&s, client, previous);
    }
    if headers.get_all("remote-email").iter().count() > 1 {
        tracing::warn!(event = "auth.sso_failed", ip = %client.ip, reason = "several_emails");
        return failed(&s, client, previous);
    }
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let Some(email) = text("remote-email").filter(|e| !e.trim().is_empty()) else {
        tracing::warn!(event = "auth.sso_failed", ip = %client.ip, reason = "no_email");
        return failed(&s, client, previous);
    };
    let name = text("remote-name");
    let ua = text("user-agent").unwrap_or_default();
    finish(
        &s,
        client,
        &ua,
        previous,
        &email,
        name.as_deref(),
        Method::Oauth,
        None,
    )
    .await
}

fn oidc_failed(id: &str, ip: IpAddr, reason: &str) {
    tracing::warn!(event = "auth.oidc_failed", provider = %id, ip = %ip, reason);
}

async fn oidc_start(State(s): State<AppState>, client: Client, Path(id): Path<String>) -> Response {
    let Some(provider) = s.oidc.get(&id).cloned() else {
        return not_found();
    };
    if let Some(res) = start_limited(&s, client.ip) {
        return res;
    }
    let pending = Pending::new(&id, crate::now());
    let redirect = public(&s, &format!("/auth/oidc/{id}/callback"));
    let Some(target) = provider.authorize_url(&redirect, &pending) else {
        oidc_failed(&id, client.ip, "not_ready");
        return sign_in_failed();
    };
    let mut res = found(target.as_str());
    res.headers_mut().append(
        header::SET_COOKIE,
        flow::pending_cookie(&pending.seal(&s.oidc.mac_key), client.https, &pending.state),
    );
    res
}

/// Over HTTPS only the `__Host-` name counts, as for the session cookie.
fn read_pending(headers: &HeaderMap, https: bool, state: &str) -> Option<String> {
    let name = flow::cookie_name(https, state);
    headers
        .get_all(header::COOKIE)
        .iter()
        .flat_map(|v| v.to_str().unwrap_or("").split(';'))
        .find_map(|part| match part.trim().split_once('=') {
            Some((n, v)) if n == name && !v.is_empty() => Some(v.to_string()),
            _ => None,
        })
}

/// Spends the pending cookie of this sign-in (named after its `state`), whatever happened.
fn with_cleared(mut res: Response, https: bool, state: &str) -> Response {
    res.headers_mut()
        .append(header::SET_COOKIE, flow::clear_pending(https, state));
    res
}

async fn oidc_callback(
    State(s): State<AppState>,
    client: Client,
    headers: HeaderMap,
    MaybeUser(previous): MaybeUser,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(provider) = s.oidc.get(&id).cloned() else {
        return not_found();
    };
    let now = crate::now();
    let https = client.https;
    let sent_state = q.get("state").cloned().unwrap_or_default();
    // Before the provider's answer is trusted, nothing about who is signed in changes: a reload of this URL after
    // signing in lands on the sign-in page, which sends a signed-in person on to their files.
    let pending = read_pending(&headers, https, &sent_state)
        .and_then(|c| flow::open(&c, &s.oidc.mac_key, now))
        .filter(|p| p.provider == id);
    let state_ok = pending
        .as_ref()
        .is_some_and(|p| crate::config::ct_eq(p.state.as_bytes(), sent_state.as_bytes()));
    let Some(pending) = pending.filter(|p| state_ok && s.oidc.first_use(&p.state, now)) else {
        oidc_failed(&id, client.ip, "state");
        return with_cleared(sign_in_failed(), https, &sent_state);
    };
    let code = q.get("code").filter(|c| !c.is_empty());
    let Some(code) = code.filter(|_| !q.contains_key("error")) else {
        oidc_failed(&id, client.ip, "provider_error");
        return with_cleared(sign_in_failed(), https, &sent_state);
    };
    let redirect = public(&s, &format!("/auth/oidc/{id}/callback"));
    let token = match provider.exchange(code, &redirect, &pending.verifier).await {
        Ok(t) => t,
        Err(error) => {
            tracing::warn!(event = "auth.oidc_failed", provider = %id, ip = %client.ip, reason = "token_exchange", error = %error);
            return with_cleared(failed(&s, client, previous), https, &sent_state);
        }
    };
    let identity = match provider.verify_id_token(&token, &pending.nonce, now).await {
        Ok(i) => i,
        Err(e) => {
            oidc_failed(&id, client.ip, e.reason());
            return with_cleared(failed(&s, client, previous), https, &sent_state);
        }
    };
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let res = finish(
        &s,
        client,
        ua,
        previous,
        &identity.email,
        identity.name.as_deref(),
        Method::Oidc,
        Some(&id),
    )
    .await;
    with_cleared(res, https, &sent_state)
}
