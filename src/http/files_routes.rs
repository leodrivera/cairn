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
        .route("/files", get(list_root).delete(delete_paths))
        .route("/files/{*path}", get(list_path))
        .route("/raw/{*path}", get(raw))
        .route("/folders/{*path}", axum::routing::post(new_folder))
        .route("/ops", axum::routing::post(run_ops))
        .route(
            "/upload/{*path}",
            axum::routing::put(upload).layer(axum::extract::DefaultBodyLimit::disable()),
        )
}

/// Files and archives served to the browser may be HTML or SVG from anyone: they must never run script here.
pub const RAW_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; sandbox";

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

#[derive(serde::Deserialize)]
struct RawQuery {
    #[serde(default)]
    download: Option<String>,
}

async fn raw(
    fu: FileUser,
    Path(raw_path): Path<String>,
    axum::extract::Query(q): axum::extract::Query<RawQuery>,
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, ApiError> {
    use axum::http::header;
    use axum::response::IntoResponse;
    use rustix::fs::{FileType, OFlags};
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    use crate::fs::raw::{ByteRange, encode_filename, etag, parse_range};

    fu.need(true, false)?;
    let path = RelPath::parse(&raw_path)?;
    let name = path.name().unwrap_or("download").to_string();
    let scope = fu.scope;
    let (file, st) = blocking(move || {
        let fd = scope.open(&path, OFlags::RDONLY)?;
        let st = rustix::fs::fstat(&fd)?;
        if FileType::from_raw_mode(st.st_mode) == FileType::Directory {
            return Err(FsError::IsAFolder);
        }
        Ok((std::fs::File::from(fd), st))
    })
    .await?;
    let size = st.st_size as u64;
    let tag = etag(&st);
    let modified =
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(st.st_mtime.max(0) as u64);
    let disposition = if q.download.is_some() {
        "attachment"
    } else {
        "inline"
    };
    let common = [
        (header::ETAG, tag.clone()),
        (header::LAST_MODIFIED, httpdate::fmt_http_date(modified)),
        (header::ACCEPT_RANGES, "bytes".to_string()),
        (header::CONTENT_SECURITY_POLICY, RAW_CSP.to_string()),
        (
            header::CONTENT_DISPOSITION,
            format!("{disposition}; filename*=UTF-8''{}", encode_filename(&name)),
        ),
    ];
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(tag.as_str())
    {
        return Ok((StatusCode::NOT_MODIFIED, common).into_response());
    }
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let Ok(range) = parse_range(range, size) else {
        return Ok((
            StatusCode::RANGE_NOT_SATISFIABLE,
            common,
            [(header::CONTENT_RANGE, format!("bytes */{size}"))],
        )
            .into_response());
    };
    let mut file = tokio::fs::File::from_std(file);
    let (status, start, len) = match range {
        ByteRange::Full => (StatusCode::OK, 0, size),
        ByteRange::Part { start, end } => (StatusCode::PARTIAL_CONTENT, start, end - start + 1),
    };
    if start > 0 {
        file.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(FsError::from)?;
    }
    let body = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file.take(len)));
    let mut res = (status, common, body).into_response();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        crate::fs::mime::guess(&name).parse().expect("static MIME"),
    );
    h.insert(header::CONTENT_LENGTH, len.into());
    if let ByteRange::Part { start, end } = range {
        h.insert(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{size}")
                .parse()
                .expect("ASCII"),
        );
    }
    Ok(res)
}

#[derive(serde::Deserialize)]
struct UploadQuery {
    #[serde(default)]
    replace: Option<String>,
}

async fn upload(
    axum::extract::State(s): axum::extract::State<AppState>,
    fu: FileUser,
    Path(raw_path): Path<String>,
    axum::extract::Query(q): axum::extract::Query<UploadQuery>,
    body: axum::body::Body,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;

    fu.need(false, true)?;
    let path = RelPath::parse(&raw_path)?;
    let replace = q.replace.is_some_and(|v| v == "1" || v == "true");
    let (_, replaced) = crate::fs::upload::upload(
        &fu.scope,
        s.db.clone(),
        &path,
        body,
        s.config.upload_max,
        replace,
        crate::now(),
    )
    .await?;
    tracing::info!(event = "file.upload", user = fu.user.id, path = %path, replaced);
    let scope = fu.scope;
    let entry = blocking(move || list::entry(&scope, &path)).await?;
    let status = if replaced {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(json!({ "entry": entry }))).into_response())
}

async fn new_folder(
    fu: FileUser,
    Path(raw_path): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;

    fu.need(false, true)?;
    let path = RelPath::parse(&raw_path)?;
    let user = fu.user.id;
    let scope = fu.scope;
    let shown = path.to_string();
    let entry = blocking(move || {
        crate::fs::edit::mkdir(&scope, &path)?;
        list::entry(&scope, &path)
    })
    .await?;
    tracing::info!(event = "file.mkdir", user, path = %shown);
    Ok((StatusCode::CREATED, Json(json!({ "entry": entry }))).into_response())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteBody {
    paths: Vec<String>,
}

/// One failure in a batch, with the same code and message a single request would get.
pub fn failure(key: &str, value: &str, e: FsError) -> Value {
    let api = ApiError::from(e);
    json!({ key: value, "error": api.code, "message": api.message })
}

async fn delete_paths(
    fu: FileUser,
    crate::http::json::StrictJson(b): crate::http::json::StrictJson<DeleteBody>,
) -> Result<Json<Value>, ApiError> {
    fu.need(false, true)?;
    if b.paths.is_empty() || b.paths.len() > 10_000 {
        return Err(ApiError::bad_request("Send between 1 and 10000 paths."));
    }
    let user = fu.user.id;
    let scope = fu.scope;
    let (deleted, failed) = blocking(move || {
        let (mut deleted, mut failed) = (0, Vec::new());
        for raw in &b.paths {
            match RelPath::parse(raw).and_then(|p| crate::fs::edit::remove(&scope, &p)) {
                Ok(()) => deleted += 1,
                Err(e) => failed.push(failure("path", raw, e)),
            }
        }
        Ok((deleted, failed))
    })
    .await?;
    tracing::info!(
        event = "file.delete",
        user,
        paths = deleted,
        failed = failed.len()
    );
    Ok(Json(json!({ "deleted": deleted, "failed": failed })))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OpItem {
    op: crate::fs::ops::Op,
    from: String,
    to: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OpsBody {
    ops: Vec<OpItem>,
}

async fn run_ops(
    fu: FileUser,
    crate::http::json::StrictJson(b): crate::http::json::StrictJson<OpsBody>,
) -> Result<Json<Value>, ApiError> {
    fu.need(false, true)?;
    if b.ops.is_empty() || b.ops.len() > 10_000 {
        return Err(ApiError::bad_request(
            "Send between 1 and 10000 operations.",
        ));
    }
    let user = fu.user.id;
    let scope = fu.scope;
    let (done, failed) = blocking(move || {
        let (mut done, mut failed) = (0, Vec::new());
        for item in &b.ops {
            let result = RelPath::parse(&item.from)
                .and_then(|from| Ok((from, RelPath::parse(&item.to)?)))
                .and_then(|(from, to)| crate::fs::ops::apply(&scope, item.op, &from, &to));
            match result {
                Ok(()) => {
                    done += 1;
                    tracing::info!(event = "file.op", user, op = ?item.op, from = %item.from, to = %item.to);
                }
                Err(e) => failed.push(failure("from", &item.from, e)),
            }
        }
        Ok((done, failed))
    })
    .await?;
    Ok(Json(json!({ "done": done, "failed": failed })))
}
