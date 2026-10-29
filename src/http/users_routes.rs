//! Users admin API: `/api/users`. Admins only.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::password::{POLICY, policy_ok};
use crate::auth::session::{self, Method};
use crate::http::AppState;
use crate::http::current_user::AdminUser;
use crate::http::error::ApiError;
use crate::http::json::{IdPath, StrictJson};
use crate::users::{self, NewUser, Perms, UserPatch};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/users", get(list).post(create))
        .route("/users/{id}", axum::routing::patch(update).delete(delete))
        .route(
            "/users/{id}/password",
            put(set_password).delete(remove_password),
        )
}

fn sso_on(s: &AppState) -> bool {
    s.config.auth.oauth.is_some() || !s.config.auth.oidc.is_empty()
}

fn weak() -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "weak_password", POLICY)
}

async fn list(State(s): State<AppState>, _: AdminUser) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "users": users::list(&s.db)? })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    email: String,
    name: String,
    #[serde(default)]
    password: Option<String>,
    scope: String,
    permissions: Perms,
}

async fn create(
    State(s): State<AppState>,
    AdminUser(by): AdminUser,
    StrictJson(b): StrictJson<Create>,
) -> Result<Response, ApiError> {
    let password_hash = match b.password {
        Some(p) if !policy_ok(&p) => return Err(weak()),
        Some(p) => Some(s.hasher.hash(p).await),
        None if !sso_on(&s) => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "password_required",
                "This server signs in with passwords only, so the user needs one.",
            ));
        }
        None => None,
    };
    let new = NewUser {
        email: b.email,
        name: b.name,
        password_hash,
        scope: b.scope,
        perms: b.permissions,
    };
    let user = users::create(&s.db, new, crate::now())?;
    tracing::info!(event = "users.created", by = by.session.user.id, user = user.id, email = %user.email);
    Ok((StatusCode::CREATED, Json(json!({ "user": user }))).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    permissions: Option<Perms>,
}

async fn update(
    State(s): State<AppState>,
    AdminUser(by): AdminUser,
    IdPath(id): IdPath,
    StrictJson(b): StrictJson<Update>,
) -> Result<Json<Value>, ApiError> {
    let before = users::by_id(&s.db, id)?.ok_or(users::UserError::NotFound)?;
    if let Some(e) = &b.email
        && users::normalize_email(e).is_none()
    {
        return Err(users::UserError::Invalid("Enter a valid e-mail address.").into());
    }
    if id == by.session.user.id
        && b.email
            .as_ref()
            .is_some_and(|e| users::normalize_email(e).as_deref() != Some(&before.email))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "own_email",
            "Nobody changes their own e-mail. Ask another admin.",
        ));
    }
    let patch = UserPatch {
        email: b.email,
        name: b.name,
        scope: b.scope,
        perms: b.permissions,
        ..Default::default()
    };
    let after = users::update(&s.db, id, patch)?;
    let (p, q) = (before.perms, after.perms);
    let lost = (p.read && !q.read)
        || (p.write && !q.write)
        || (p.share && !q.share)
        || (p.admin && !q.admin);
    let email_changed = before.email != after.email;
    if email_changed || before.scope != after.scope || lost {
        let keep = (id == by.session.user.id).then_some(by.token.as_str());
        session::delete_for_user(&s.db, id, keep)?;
    }
    if email_changed {
        tracing::warn!(event = "users.email_reassigned", by = by.session.user.id, user = id, from = %before.email, to = %after.email);
    }
    tracing::info!(event = "users.updated", by = by.session.user.id, user = id);
    Ok(Json(json!({ "user": after })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetPassword {
    password: String,
}

async fn set_password(
    State(s): State<AppState>,
    AdminUser(by): AdminUser,
    IdPath(id): IdPath,
    StrictJson(b): StrictJson<SetPassword>,
) -> Result<Json<Value>, ApiError> {
    if !policy_ok(&b.password) {
        return Err(weak());
    }
    users::by_id(&s.db, id)?.ok_or(users::UserError::NotFound)?;
    let hash = s.hasher.hash(b.password).await;
    users::set_password(&s.db, id, Some(hash))?;
    let keep = (id == by.session.user.id).then_some(by.token.as_str());
    session::delete_for_user(&s.db, id, keep)?;
    tracing::info!(
        event = "auth.password_reset",
        by = by.session.user.id,
        user = id
    );
    Ok(Json(json!({ "ok": true })))
}

async fn remove_password(
    State(s): State<AppState>,
    AdminUser(by): AdminUser,
    IdPath(id): IdPath,
) -> Result<Json<Value>, ApiError> {
    if !sso_on(&s) {
        return Err(ApiError::conflict(
            "password_only",
            "This server signs in with passwords only, so the password cannot be removed.",
        ));
    }
    users::by_id(&s.db, id)?.ok_or(users::UserError::NotFound)?;
    users::set_password(&s.db, id, None)?;
    let keep = (id == by.session.user.id).then_some(by.token.as_str());
    session::delete_for_user_method(&s.db, id, Method::Password, keep)?;
    tracing::info!(
        event = "auth.password_removed",
        by = by.session.user.id,
        user = id
    );
    Ok(Json(json!({ "ok": true })))
}

async fn delete(
    State(s): State<AppState>,
    AdminUser(by): AdminUser,
    IdPath(id): IdPath,
) -> Result<Response, ApiError> {
    users::delete(&s.db, id)?;
    tracing::info!(event = "users.deleted", by = by.session.user.id, user = id);
    Ok(StatusCode::NO_CONTENT.into_response())
}
