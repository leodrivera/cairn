//! The public side of share links: everything under `/s/{id}`, with no session. The page is the app; `info`, `list`,
//! `raw` and `zip` serve the shared file or folder. They live under the link's own path, not `/api`, because the
//! unlock cookie of a link with a password is scoped to `/s/{id}`.

use std::collections::HashMap;

use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, get, post};
use axum::{Json, Router, middleware};
use serde_json::json;

use crate::auth::ratelimit::ip_key;
use crate::fs::path::RelPath;
use crate::fs::{FsError, list};
use crate::http::AppState;
use crate::http::assets;
use crate::http::client::Client;
use crate::http::csrf;
use crate::http::error::ApiError;
use crate::http::files_routes::{blocking, serve_file, serve_zip};
use crate::http::json::StrictJson;
use crate::http::sso_routes::not_found;
use crate::shares::{self, Live, Target};

pub fn routes(state: AppState) -> Router<AppState> {
    // Other methods get the same plain 404 as any unknown /s path.
    let page = |m: MethodRouter<AppState>| m.fallback(page_not_found);
    // The one state change here: like every POST under /api it needs X-Cairn, and a small body.
    let unlock_route = post(unlock)
        .route_layer(middleware::from_fn_with_state(state, csrf::require_header))
        .route_layer(DefaultBodyLimit::max(1 << 20));
    Router::new()
        .route("/s/{id}", page(get(app_page)))
        .route("/s/{id}/info", page(get(info)))
        .route("/s/{id}/info/{*path}", page(get(info_item)))
        .route("/s/{id}/unlock", page(unlock_route))
        .route("/s/{id}/list", page(get(list_folder)))
        .route("/s/{id}/list/{*path}", page(get(list_folder)))
        .route("/s/{id}/raw", page(get(raw)))
        .route("/s/{id}/raw/{*path}", page(get(raw)))
        .route("/s/{id}/zip", page(get(zip)))
        .route("/s/{id}/zip/{*path}", page(get(zip)))
}

async fn page_not_found() -> Response {
    not_found()
}

/// The link id and the path beneath the shared folder. An id that cannot be one is the plain page 404.
struct At {
    id: String,
    path: String,
}

impl<S: Send + Sync> FromRequestParts<S> for At {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<At, Response> {
        let Path(mut params) = Path::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .map_err(|_| not_found())?;
        let id = params.remove("id").unwrap_or_default();
        if !shares::is_id(&id) {
            return Err(not_found());
        }
        Ok(At {
            id,
            path: params.remove("path").unwrap_or_default(),
        })
    }
}

fn gone() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "share_gone",
        "This link no longer works. Ask the person who sent it for a new one.",
    )
}

fn locked() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "share_locked",
        "This link needs a password.",
    )
}

/// The link, when it works, whether or not the visitor has unlocked it.
async fn find(s: &AppState, id: &str) -> Result<Live, ApiError> {
    let (db, root, data_id, fallback, id) = (
        s.db.clone(),
        s.config.root.clone(),
        s.reserved(),
        s.force_fallback,
        id.to_string(),
    );
    let found = tokio::task::spawn_blocking(move || {
        shares::resolve(&db, &root, data_id, fallback, &id, crate::now())
    })
    .await
    .map_err(|_| ApiError::internal())?;
    found.ok_or_else(gone)
}

/// The link, when it works and the visitor may open it: no password, or a valid unlock cookie for this password.
async fn live(s: &AppState, id: &str, headers: &HeaderMap) -> Result<Live, ApiError> {
    let live = find(s, id).await?;
    if let Some(hash) = &live.row.password_hash
        && !shares::read_unlock(headers, id, hash, &s.share_key, crate::now())
    {
        return Err(locked());
    }
    Ok(live)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UnlockBody {
    password: String,
}

/// A wrong password is 403, never 401: only a failed sign-in is 401, for Fail2ban.
async fn unlock(
    State(s): State<AppState>,
    at: At,
    client: Client,
    StrictJson(b): StrictJson<UnlockBody>,
) -> Result<Response, ApiError> {
    let live = find(&s, &at.id).await?;
    let Some(hash) = live.row.password_hash else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let now = crate::now();
    let key = format!("{}|{}", at.id, ip_key(client.ip));
    let limiter = &s.limiters.share_password;
    // Counted before the slow check, so tries sent in parallel cannot all pass it; a right password clears it.
    if let Some(wait) = limiter.try_count(client.ip, &key, now) {
        tracing::warn!(event = "http.rate_limited", route = "share_password", ip = %client.ip);
        return Err(ApiError::too_many(wait));
    }
    if !s.hasher.verify(b.password, Some(hash.clone())).await {
        tracing::warn!(event = "share.password_failed", ip = %client.ip);
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "wrong_link_password",
            "That password is not right.",
        ));
    }
    limiter.succeeded(&key);
    let cookie = shares::unlock_cookie(&at.id, &hash, &s.share_key, now, client.https);
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response())
}

/// Where a path beneath the link leads: a file link has nothing beneath it.
fn place(target: Target, sub: &str) -> Result<(crate::fs::scope::Scope, RelPath), ApiError> {
    let sub = RelPath::parse(sub)?;
    match target {
        Target::File { scope, path } if sub.is_root() => Ok((scope, path)),
        Target::File { .. } => Err(FsError::NotAFolder.into()),
        Target::Folder { scope } => Ok((scope, sub)),
    }
}

/// What a link shows may be behind a password: no shared cache may keep it.
fn private(mut res: Response) -> Response {
    res.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    res
}

/// The last segment of the link's path, as people read it; `None` for the server root.
fn link_name(stored: &str) -> Option<String> {
    RelPath::parse(stored)
        .ok()
        .and_then(|p| p.name_lossy().map(|n| n.into_owned()))
}

async fn app_page(_: At, headers: HeaderMap) -> Response {
    assets::index(&headers)
}

async fn info(State(s): State<AppState>, at: At, headers: HeaderMap) -> Result<Response, ApiError> {
    let Live {
        row,
        creator,
        target,
    } = live(&s, &at.id, &headers).await?;
    let name = link_name(&row.path).unwrap_or_else(|| "/".into());
    let entry = blocking(move || match &target {
        Target::File { scope, path } => list::entry(scope, path),
        Target::Folder { scope } => {
            let mut e = list::entry(scope, &RelPath::parse("")?)?;
            e.name = name;
            Ok(e)
        }
    })
    .await?;
    shares::hit(&s.db, &row.id)?;
    Ok(private(
        Json(json!({
            "entry": entry,
            "download": row.download,
            "expiresAt": row.expires_at,
            "sharedBy": creator.name,
        }))
        .into_response(),
    ))
}

/// One item beneath a shared folder, for the page of a file inside it. Not counted in `hits`: the page was.
async fn info_item(
    State(s): State<AppState>,
    at: At,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let live = live(&s, &at.id, &headers).await?;
    let Target::Folder { scope } = live.target else {
        return Err(FsError::NotAFolder.into());
    };
    let path = RelPath::parse(&at.path)?;
    let entry = blocking(move || list::entry(&scope, &path)).await?;
    Ok(private(Json(json!({ "entry": entry })).into_response()))
}

async fn list_folder(
    State(s): State<AppState>,
    at: At,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let live = live(&s, &at.id, &headers).await?;
    let Target::Folder { scope } = live.target else {
        return Err(FsError::NotAFolder.into());
    };
    let path = RelPath::parse(&at.path)?;
    let shown = path.to_string();
    let entries = blocking(move || list::list(&scope, &path)).await?;
    Ok(private(
        Json(json!({ "path": shown, "entries": entries })).into_response(),
    ))
}

#[derive(serde::Deserialize)]
struct RawQuery {
    #[serde(default)]
    download: Option<String>,
}

/// With downloads off the file is still shown inline: anything a browser shows can be saved anyway.
async fn raw(
    State(s): State<AppState>,
    at: At,
    Query(q): Query<RawQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let live = live(&s, &at.id, &headers).await?;
    let attachment = q.download.is_some() && live.row.download;
    let (scope, path) = place(live.target, &at.path)?;
    Ok(private(
        serve_file(scope, path, &headers, attachment).await?,
    ))
}

async fn zip(
    State(s): State<AppState>,
    client: Client,
    at: At,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let live = live(&s, &at.id, &headers).await?;
    if !live.row.download {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "download_off",
            "Downloads are off for this link.",
        ));
    }
    let Target::Folder { scope } = live.target else {
        return Err(FsError::NotAFolder.into());
    };
    let path = RelPath::parse(&at.path)?;
    let name = link_name(&live.row.path).unwrap_or_else(|| "files".into());
    Ok(private(
        serve_zip(&s.zips, client, scope, path, None, &name).await?,
    ))
}
