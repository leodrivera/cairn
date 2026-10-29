//! Who is signed in: the `load` middleware reads the session cookie once per request; extractors read the result.

use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::auth::session::{self, KEEP_SECS, Session};
use crate::http::AppState;
use crate::http::client::{Client, Peer};
use crate::http::error::ApiError;

pub const COOKIE: &str = "cairn_session";
/// Over HTTPS: the browser refuses it from any other host, so a sibling subdomain cannot plant a session.
pub const COOKIE_HTTPS: &str = "__Host-cairn_session";

#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub session: Session,
    pub token: String,
}

/// The signed-in user, if any. Never rejects.
pub struct MaybeUser(pub Option<CurrentUser>);
/// Rejects admins-only routes for everyone else.
pub struct AdminUser(pub CurrentUser);

#[derive(Clone)]
struct Loaded(Option<CurrentUser>);

pub fn read_token(headers: &HeaderMap) -> Option<String> {
    let mut plain = None;
    for value in headers.get_all(header::COOKIE) {
        for part in value.to_str().unwrap_or("").split(';') {
            match part.trim().split_once('=') {
                Some((COOKIE_HTTPS, v)) if !v.is_empty() => return Some(v.to_string()),
                Some((COOKIE, v)) if !v.is_empty() => plain = Some(v.to_string()),
                _ => {}
            }
        }
    }
    plain
}

pub fn set_cookie(token: &str, keep: bool, https: bool) -> HeaderValue {
    let name = if https { COOKIE_HTTPS } else { COOKIE };
    let mut c = format!("{name}={token}; Path=/; HttpOnly; SameSite=Lax");
    if keep {
        c.push_str(&format!("; Max-Age={KEEP_SECS}"));
    }
    if https {
        c.push_str("; Secure");
    }
    HeaderValue::from_str(&c).expect("cookie is ASCII")
}

pub fn clear_cookie(https: bool) -> HeaderValue {
    let name = if https { COOKIE_HTTPS } else { COOKIE };
    let secure = if https { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{name}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}"
    ))
    .expect("ASCII")
}

pub async fn load(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let token = read_token(req.headers());
    let current = match &token {
        Some(token) => match session::lookup(&state.db, token, crate::now()) {
            Ok(found) => found.map(|session| CurrentUser {
                session,
                token: token.clone(),
            }),
            Err(e) => {
                tracing::error!(event = "db.error", error = %e);
                None
            }
        },
        None => None,
    };
    let renewed = current
        .as_ref()
        .filter(|c| c.session.renewed)
        .map(|c| c.token.clone());
    let peer = req.extensions().get::<Peer>().map(|p| p.0.ip());
    let https = Client::from_parts(peer, req.headers(), &state.config.trusted_proxies).https;
    req.extensions_mut().insert(Loaded(current));
    let mut res = next.run(req).await;
    if let Some(token) = renewed
        && !res.headers().contains_key(header::SET_COOKIE)
    {
        res.headers_mut()
            .append(header::SET_COOKIE, set_cookie(&token, true, https));
    }
    res
}

fn loaded(parts: &Parts) -> Option<CurrentUser> {
    parts.extensions.get::<Loaded>().and_then(|l| l.0.clone())
}

impl<S: Send + Sync> FromRequestParts<S> for MaybeUser {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(MaybeUser(loaded(parts)))
    }
}

impl<S: Send + Sync> FromRequestParts<S> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, ApiError> {
        loaded(parts).ok_or_else(ApiError::signed_out)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, ApiError> {
        let user = loaded(parts).ok_or_else(ApiError::signed_out)?;
        if !user.session.user.perms.admin {
            return Err(ApiError::forbidden());
        }
        Ok(AdminUser(user))
    }
}
