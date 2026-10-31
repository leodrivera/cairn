//! `/api/g/{id}/…`: the file routes inside a grant. The request is handed to the same handlers with the grant as
//! its scope; only the file routes are reachable this way, so nothing received can be shared on.

use axum::extract::{Path, Request, State};
use axum::http::Uri;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::{Json, Router};
use serde_json::json;
use tower::ServiceExt;

use crate::http::AppState;
use crate::http::current_user::{self, CurrentUser};
use crate::http::error::ApiError;
use crate::http::files_routes::{self, GrantCall, grant_gone};

pub fn routes(state: AppState) -> Router<AppState> {
    let inner = files_routes::routes()
        .layer(axum::extract::DefaultBodyLimit::max(1 << 20))
        .with_state(state);
    Router::new().route("/g/{id}", get(info)).route(
        "/g/{id}/{*rest}",
        any(move |p: Path<(String, String)>, req: Request| forward(inner.clone(), p, req)),
    )
}

async fn info(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = me.session.user;
    let viewer = user.clone();
    let live = tokio::task::spawn_blocking(move || {
        crate::grants::resolve(
            &s.db,
            &s.config.root,
            s.reserved(),
            s.force_fallback,
            &id,
            user.id,
            crate::now(),
        )
    })
    .await
    .map_err(|_| ApiError::internal())?
    .ok_or_else(grant_gone)?;
    Ok(Json(
        json!({ "grant": crate::http::grants_routes::received_json(&live, &viewer) }),
    ))
}

/// A fresh request for the file routes: the path after the grant, the same method, headers, body and query, who is
/// signed in, and the grant. The outer route's parameters stay behind, so the handlers see only their own.
async fn forward(
    inner: Router,
    Path((id, rest)): Path<(String, String)>,
    req: Request,
) -> Response {
    let (parts, body) = req.into_parts();
    let mut path = format!("/{rest}");
    if let Some(q) = parts.uri.query() {
        path = format!("{path}?{q}");
    }
    let Ok(uri) = path.parse::<Uri>() else {
        return ApiError::not_found().into_response();
    };
    let mut fresh = Request::new(body);
    *fresh.method_mut() = parts.method;
    *fresh.uri_mut() = uri;
    *fresh.version_mut() = parts.version;
    *fresh.headers_mut() = parts.headers;
    current_user::carry(&parts.extensions, fresh.extensions_mut());
    fresh.extensions_mut().insert(GrantCall(id));
    match inner.oneshot(fresh).await {
        Ok(res) => res,
        Err(never) => match never {},
    }
}
