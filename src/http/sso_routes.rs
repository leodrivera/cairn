//! Sign-in with a provider: page navigations outside `/api`. `GET /auth/oauth/{provider}` starts a broker sign-in.

use std::net::IpAddr;

use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Router, middleware};
use serde_json::{Value, json};

use crate::http::AppState;
use crate::http::client::Client;
use crate::http::current_user;
use crate::http::error::ApiError;

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/auth/oauth/{provider}", get(oauth_start))
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
            out.push(json!({ "name": p.name, "href": format!("/auth/oauth/{}", p.id) }));
        }
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
    if !broker.providers().iter().any(|p| p.id == provider) {
        return not_found();
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
