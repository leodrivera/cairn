//! Sign-in and profile: `/api/config`, `/api/auth/login`, `/api/auth/logout`, `/api/me`, `/api/me/password`.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::captcha::CaptchaFailure;
use crate::auth::password::{POLICY, policy_ok};
use crate::auth::session::{self, Method};
use crate::http::AppState;
use crate::http::client::Client;
use crate::http::current_user::{CurrentUser, MaybeUser, clear_cookie, set_cookie};
use crate::http::error::ApiError;
use crate::http::json::{StrictJson, some};
use crate::users::{self, UserPatch};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/config", get(config))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/me", get(me).patch(patch_me))
        .route("/me/password", put(change_password))
}

async fn config(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    let auth = &s.config.auth;
    let mut methods = Vec::new();
    if auth.password {
        methods.push("password");
    }
    if auth.oauth.is_some() {
        methods.push("oauth");
    }
    if !auth.oidc.is_empty() {
        methods.push("oidc");
    }
    let captcha = s
        .captcha
        .as_ref()
        .map(|c| json!({ "provider": c.vendor_name(), "siteKey": c.site_key() }));
    let mut body = json!({ "authMethods": methods, "captcha": captcha, "providers": [] });
    if users::count(&s.db)? == 0 {
        body["setupNeeded"] = json!(true);
    }
    Ok(Json(body))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Login {
    email: String,
    password: String,
    #[serde(default)]
    captcha_token: Option<String>,
    #[serde(default)]
    keep: bool,
}

fn wrong_credentials() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "wrong_credentials",
        "Wrong e-mail or password.",
    )
}

fn captcha_error(f: CaptchaFailure) -> ApiError {
    let code = match f {
        CaptchaFailure::Missing => "captcha_missing",
        CaptchaFailure::Invalid => "captcha_invalid",
        CaptchaFailure::Hostname => "captcha_hostname",
        CaptchaFailure::Unreachable => "captcha_unreachable",
    };
    ApiError::new(StatusCode::BAD_REQUEST, code, f.message())
}

async fn login(
    State(s): State<AppState>,
    client: Client,
    headers: HeaderMap,
    StrictJson(b): StrictJson<Login>,
) -> Result<Response, ApiError> {
    if !s.config.auth.password {
        return Err(ApiError::not_found());
    }
    let now = crate::now();
    let email = users::normalize_email(&b.email);
    // The limiter key for a malformed address is the text as typed, so it is still counted.
    let key = email
        .clone()
        .unwrap_or_else(|| b.email.trim().to_lowercase().chars().take(254).collect());
    if let Some(wait) = s.limiters.login.blocked(client.ip, &key, now) {
        tracing::warn!(event = "http.rate_limited", route = "login", ip = %client.ip, email = %key);
        return Err(ApiError::too_many(wait));
    }
    if let Some(captcha) = &s.captcha
        && let Err(f) = captcha.verify(b.captcha_token.as_deref(), client.ip).await
    {
        tracing::warn!(event = "auth.captcha_failed", ip = %client.ip, email = %key, reason = f.reason());
        return Err(captcha_error(f));
    }
    let found = match &email {
        Some(e) => users::by_email(&s.db, e)?,
        None => None,
    };
    let user_exists = found.is_some();
    let hash = found.as_ref().and_then(|(_, h)| h.clone());
    // Over-long passwords are checked against the dummy, so they cost the same time and never match.
    let usable = b.password.chars().count() <= 256;
    let ok = s
        .hasher
        .verify(b.password, hash.clone().filter(|_| usable))
        .await;
    let Some((user, _)) = found.filter(|_| ok) else {
        s.limiters.login.failed(client.ip, &key, now);
        let reason = found_reason(email.is_some() && user_exists, hash.is_some());
        tracing::warn!(event = "auth.login_failed", ip = %client.ip, email = %key, reason);
        return Err(wrong_credentials());
    };
    s.limiters.login.succeeded(&key);
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = session::create(
        &s.db,
        user.id,
        Method::Password,
        b.keep,
        &session::label_from_user_agent(ua),
        now,
    )?;
    tracing::info!(event = "auth.login", user = user.id, email = %user.email, ip = %client.ip, method = "password");
    Ok((
        [(header::SET_COOKIE, set_cookie(&token, b.keep, client.https))],
        Json(json!({ "user": user })),
    )
        .into_response())
}

fn found_reason(user: bool, hash: bool) -> &'static str {
    match (user, hash) {
        (false, _) => "unknown_user",
        (true, false) => "no_password",
        (true, true) => "bad_password",
    }
}

async fn logout(
    State(s): State<AppState>,
    client: Client,
    MaybeUser(current): MaybeUser,
) -> Result<Response, ApiError> {
    if let Some(c) = current {
        session::delete(&s.db, &c.token)?;
        tracing::info!(
            event = "auth.logout",
            user = c.session.user.id,
            method = c.session.method.as_str()
        );
    }
    Ok((
        [(header::SET_COOKIE, clear_cookie(client.https))],
        Json(json!({ "next": null })),
    )
        .into_response())
}

async fn me(State(s): State<AppState>, MaybeUser(current): MaybeUser) -> Json<Value> {
    let Some(c) = current else {
        return Json(json!({ "user": null }));
    };
    let mut body = json!({ "user": c.session.user, "method": c.session.method.as_str() });
    if c.session.user.perms.admin {
        let off = s.config.auth.password && s.captcha.is_none();
        body["security"] = json!({ "captchaDisabled": off });
    }
    Json(body)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MePatch {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, deserialize_with = "some")]
    lang: Option<Option<String>>,
    #[serde(default, deserialize_with = "some")]
    theme: Option<Option<String>>,
    #[serde(default, deserialize_with = "some")]
    view: Option<Option<String>>,
}

async fn patch_me(
    State(s): State<AppState>,
    current: CurrentUser,
    StrictJson(b): StrictJson<MePatch>,
) -> Result<Json<Value>, ApiError> {
    let patch = UserPatch {
        name: b.name,
        lang: b.lang,
        theme: b.theme,
        view: b.view,
        ..Default::default()
    };
    let user = users::update(&s.db, current.session.user.id, patch)?;
    Ok(Json(json!({ "user": user })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordChange {
    current: String,
    next: String,
}

async fn change_password(
    State(s): State<AppState>,
    client: Client,
    current: CurrentUser,
    StrictJson(b): StrictJson<PasswordChange>,
) -> Result<Json<Value>, ApiError> {
    let user = &current.session.user;
    let stored = users::by_email(&s.db, &user.email)?.and_then(|(_, h)| h);
    let Some(stored) = stored else {
        return Err(ApiError::conflict(
            "no_password",
            "Your account has no password.",
        ));
    };
    let now = crate::now();
    let key = format!("user:{}", user.id);
    if let Some(wait) = s.limiters.password_change.blocked(client.ip, &key, now) {
        tracing::warn!(event = "http.rate_limited", route = "me_password", user = user.id, ip = %client.ip);
        return Err(ApiError::too_many(wait));
    }
    if b.current.chars().count() > 256 || !s.hasher.verify(b.current, Some(stored)).await {
        s.limiters.password_change.failed(client.ip, &key, now);
        // 400, not 401: a slip inside the app must not feed Fail2ban.
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "wrong_password",
            "The current password is wrong.",
        ));
    }
    if !policy_ok(&b.next) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "weak_password",
            POLICY,
        ));
    }
    s.limiters.password_change.succeeded(&key);
    let hash = s.hasher.hash(b.next).await;
    users::set_password(&s.db, user.id, Some(hash))?;
    session::delete_for_user(&s.db, user.id, Some(&current.token))?;
    tracing::info!(event = "auth.password_changed", user = user.id);
    Ok(Json(json!({ "ok": true })))
}
