//! File routes from spec section 5.2. Every path goes through the user's `Scope`.

use axum::extract::{FromRequestParts, Path};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::fs::path::RelPath;
use crate::fs::scope::Scope;
use crate::fs::{FsError, list};
use crate::http::AppState;
use crate::http::current_user::CurrentUser;
use crate::http::error::ApiError;
use crate::users::User;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/files", get(list_root))
        .route("/files/{*path}", get(list_path))
}

/// The signed-in user and their scope, opened for this request.
pub struct FileUser {
    pub user: User,
    pub scope: Scope,
}

impl FileUser {
    pub fn need(&self, read: bool, write: bool) -> Result<(), ApiError> {
        let p = self.user.perms;
        if (read && !p.read) || (write && !p.write) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "forbidden",
                "You do not have permission to do this here.",
            ));
        }
        Ok(())
    }
}

impl FromRequestParts<AppState> for FileUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let current = CurrentUser::from_request_parts(parts, state).await?;
        let user = current.session.user;
        let scope =
            Scope::new(&state.config.root, &user.scope, state.data_id).map_err(|e| match e {
                FsError::NotFound | FsError::NotAFolder => ApiError::new(
                    StatusCode::NOT_FOUND,
                    "scope_missing",
                    "Your folder does not exist on the server. Ask an admin to check your scope.",
                ),
                other => other.into(),
            })?;
        let scope = if state.force_fallback {
            scope.with_fallback()
        } else {
            scope
        };
        Ok(FileUser { user, scope })
    }
}

/// Runs filesystem work off the async threads.
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, FsError> + Send + 'static,
) -> Result<T, ApiError> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result.map_err(ApiError::from),
        Err(_) => Err(ApiError::internal()),
    }
}

async fn list_root(fu: FileUser) -> Result<Json<Value>, ApiError> {
    list_at(fu, String::new()).await
}

async fn list_path(fu: FileUser, Path(path): Path<String>) -> Result<Json<Value>, ApiError> {
    list_at(fu, path).await
}

async fn list_at(fu: FileUser, raw: String) -> Result<Json<Value>, ApiError> {
    fu.need(true, false)?;
    let path = RelPath::parse(&raw)?;
    let shown = path.to_string();
    let scope = fu.scope;
    let entries = blocking(move || list::list(&scope, &path)).await?;
    Ok(Json(json!({ "path": shown, "entries": entries })))
}
