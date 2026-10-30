//! Share link management: `/api/shares`. Creating needs `share`; each person manages their own links, admins all.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::fs::path::RelPath;
use crate::http::AppState;
use crate::http::current_user::CurrentUser;
use crate::http::error::ApiError;
use crate::http::files_routes::{FileUser, blocking};
use crate::http::json::{StrictJson, some};
use crate::shares::{self, ShareRow};
use crate::users::{self, User};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/shares", get(list).post(create))
        .route("/shares/{id}", patch(update).delete(delete))
}

fn invalid(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid", message)
}

fn no_such_link() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", "No such share link.")
}

fn check_password(p: &str) -> Result<(), ApiError> {
    let n = p.chars().count();
    if n == 0 || n > 256 {
        return Err(invalid("The link password needs 1 to 256 characters."));
    }
    Ok(())
}

/// The link as the API shows it to `viewer`: the path without the viewer's scope when it lies inside it.
pub fn share_json(s: &AppState, row: &ShareRow, owner: &User, viewer: &User) -> Value {
    let url = s
        .config
        .public_url
        .as_ref()
        .map(|u| format!("{}/s/{}", u.origin().ascii_serialization(), row.id));
    let path = match shares::split_scope(&row.path, &viewer.scope) {
        Some(rel) => format!("/{rel}"),
        None => row.path.clone(),
    };
    let name = RelPath::parse(&row.path)
        .ok()
        .and_then(|p| p.name_lossy().map(|n| n.into_owned()))
        .unwrap_or_else(|| "/".into());
    json!({
        "id": row.id,
        "url": url,
        "path": path,
        "name": name,
        "expiresAt": row.expires_at,
        "hasPassword": row.password_hash.is_some(),
        "download": row.download,
        "hits": row.hits,
        "createdAt": row.created_at,
        "owner": { "id": owner.id, "name": owner.name },
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Create {
    path: String,
    #[serde(default)]
    expires_days: Option<i64>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default = "yes")]
    download: bool,
}

fn yes() -> bool {
    true
}

async fn create(
    State(s): State<AppState>,
    fu: FileUser,
    StrictJson(b): StrictJson<Create>,
) -> Result<Response, ApiError> {
    if !fu.user.perms.share || !fu.user.perms.read {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "You do not have permission to do this here.",
        ));
    }
    let now = crate::now();
    let rel = RelPath::parse(&b.path)?;
    let expires_at = shares::expiry(b.expires_days, now).map_err(invalid)?;
    if let Some(p) = &b.password {
        check_password(p)?;
    }
    let scope = fu.scope;
    let check = rel.clone();
    blocking(move || scope.open(&check, rustix::fs::OFlags::PATH).map(|_| ())).await?;
    let password_hash = match b.password {
        Some(p) => Some(s.hasher.hash(p).await),
        None => None,
    };
    let row = ShareRow {
        id: shares::new_id(),
        user_id: fu.user.id,
        path: shares::root_path(&fu.user.scope, &rel),
        password_hash,
        download: b.download,
        expires_at,
        created_at: now,
        hits: 0,
    };
    shares::insert(&s.db, &row, now)?;
    tracing::info!(
        event = "share.create",
        user = fu.user.id,
        path = %row.path,
        expires_days = b.expires_days,
        password = row.password_hash.is_some(),
        download = row.download,
    );
    let body = json!({ "share": share_json(&s, &row, &fu.user, &fu.user) });
    Ok((StatusCode::CREATED, Json(body)).into_response())
}

async fn list(State(s): State<AppState>, me: CurrentUser) -> Result<Json<Value>, ApiError> {
    let viewer = me.session.user;
    let who = (!viewer.perms.admin).then_some(viewer.id);
    let rows = shares::list(&s.db, who, crate::now())?;
    let mut owners: std::collections::HashMap<i64, User> = std::collections::HashMap::new();
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let owner = match owners.entry(row.user_id) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => match users::by_id(&s.db, row.user_id)?
            {
                Some(u) => e.insert(u),
                None => continue,
            },
        };
        out.push(share_json(&s, row, owner, &viewer));
    }
    Ok(Json(json!({ "shares": out })))
}

/// The link, when the viewer may manage it: their own, or any for an admin. Others get the same 404 as a missing one.
fn owned(s: &AppState, viewer: &User, id: &str) -> Result<(ShareRow, User), ApiError> {
    let row = shares::get(&s.db, id)?
        .filter(|r| r.expires_at.is_none_or(|at| at > crate::now()))
        .filter(|r| viewer.perms.admin || r.user_id == viewer.id)
        .ok_or_else(no_such_link)?;
    let owner = users::by_id(&s.db, row.user_id)?.ok_or_else(no_such_link)?;
    Ok((row, owner))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Update {
    #[serde(default, deserialize_with = "some")]
    expires_days: Option<Option<i64>>,
    #[serde(default, deserialize_with = "some")]
    password: Option<Option<String>>,
    #[serde(default)]
    download: Option<bool>,
}

async fn update(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<String>,
    StrictJson(b): StrictJson<Update>,
) -> Result<Json<Value>, ApiError> {
    let viewer = me.session.user;
    let (row, owner) = owned(&s, &viewer, &id)?;
    let now = crate::now();
    let expires_at = match b.expires_days {
        Some(days) => Some(shares::expiry(days, now).map_err(invalid)?),
        None => None,
    };
    let password_hash = match b.password {
        Some(Some(p)) => {
            check_password(&p)?;
            Some(Some(s.hasher.hash(p).await))
        }
        Some(None) => Some(None),
        None => None,
    };
    shares::update(&s.db, &id, expires_at, password_hash, b.download, now)?;
    tracing::info!(event = "share.update", user = viewer.id, path = %row.path);
    let row = shares::get(&s.db, &id)?.ok_or_else(no_such_link)?;
    Ok(Json(
        json!({ "share": share_json(&s, &row, &owner, &viewer) }),
    ))
}

async fn delete(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let viewer = me.session.user;
    let (row, _) = owned(&s, &viewer, &id)?;
    shares::delete(&s.db, &id, crate::now())?;
    tracing::info!(event = "share.delete", user = viewer.id, path = %row.path);
    Ok(StatusCode::NO_CONTENT)
}
