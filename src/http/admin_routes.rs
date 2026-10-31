//! Admin screens: `PUT /api/settings` and `GET /api/server`.

use axum::extract::State;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::config::{CaptchaConfig, CaptchaVendor, LogLevel};
use crate::http::AppState;
use crate::http::auth_routes::auth_methods;
use crate::http::current_user::AdminUser;
use crate::http::error::ApiError;
use crate::http::json::StrictJson;
use crate::settings::{self, Settings};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/settings", put(save_settings))
        .route("/server", get(server))
}

async fn save_settings(
    State(s): State<AppState>,
    AdminUser(by): AdminUser,
    StrictJson(b): StrictJson<Settings>,
) -> Result<Json<Value>, ApiError> {
    if b.trash_days.is_some() && s.config.trash_days.is_some() {
        return Err(ApiError::bad_request(
            "CAIRN_TRASH_DAYS sets the trash days on this server, so they cannot be changed here.",
        ));
    }
    settings::save(&s.db, &b)?;
    tracing::info!(event = "settings.updated", by = by.session.user.id);
    Ok(Json(json!({ "settings": b })))
}

/// The environment, read-only, for the Server screen. Never a secret or an internal URL.
async fn server(State(s): State<AppState>, _: AdminUser) -> Json<Value> {
    let c = &s.config;
    let captcha = match &c.captcha {
        CaptchaConfig::None => "none",
        CaptchaConfig::On { vendor, .. } => match vendor {
            CaptchaVendor::Recaptcha => "recaptcha",
            CaptchaVendor::Turnstile => "turnstile",
            CaptchaVendor::Hcaptcha => "hcaptcha",
        },
    };
    let log = match c.log {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    };
    let proxies: Vec<String> = c.trusted_proxies.iter().map(|n| n.to_string()).collect();
    Json(json!({
        "root": c.root.to_string_lossy(),
        "publicUrl": c.public_url.as_ref().map(|u| u.as_str()),
        "addr": c.addr.to_string(),
        "authMethods": auth_methods(c),
        "captcha": captcha,
        "uploadMax": c.upload_max.unwrap_or(0),
        "editMax": c.edit_max,
        "threads": c.threads,
        "trustedProxies": proxies,
        "log": log,
        "trash": trash_info(&s),
    }))
}

fn trash_info(s: &AppState) -> Value {
    let (days, from_env) = crate::trash::days(&s.config, &s.db);
    let size: i64 =
        s.db.with(|c| c.query_row("SELECT coalesce(sum(size), 0) FROM trash", [], |r| r.get(0)))
            .unwrap_or(0);
    json!({ "days": days, "fromEnv": from_env, "size": size, "taken": s.trash.taken() })
}
