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
        .route("/zip", get(zip_root).post(zip_root_items))
        .route("/zip/{*path}", get(zip_path).post(zip_path_items))
        .route(
            "/text/{*path}",
            axum::routing::put(save_text).layer(axum::extract::DefaultBodyLimit::disable()),
        )
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
            Scope::new(&state.config.root, &user.scope, state.reserved()).map_err(|e| match e {
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
    let admin = fu.user.perms.admin;
    let scope = fu.scope;
    let (entries, space) = blocking(move || {
        let entries = list::list(&scope, &path)?;
        // Disk use is for admins only; a filesystem that cannot report it just leaves it out.
        let space = admin.then(|| scope.space()).and_then(|r| {
            r.inspect_err(|e| tracing::debug!(error = ?e, "no disk space figures"))
                .ok()
        });
        Ok((entries, space))
    })
    .await?;
    let mut body = json!({ "path": shown, "entries": entries });
    if let Some((used, total)) = space {
        body["space"] = json!({ "used": used, "total": total });
    }
    Ok(Json(body))
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
    fu.need(true, false)?;
    let path = RelPath::parse(&raw_path)?;
    serve_file(fu.scope, path, &headers, q.download.is_some()).await
}

/// One file as a download: `Range`, `If-None-Match`, the strict CSP, inline or as an attachment.
pub(crate) async fn serve_file(
    scope: Scope,
    path: RelPath,
    headers: &axum::http::HeaderMap,
    attachment: bool,
) -> Result<axum::response::Response, ApiError> {
    use axum::http::header;
    use axum::response::IntoResponse;
    use rustix::fs::{FileType, OFlags};
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    use crate::fs::raw::{ByteRange, encode_filename, etag, parse_range};

    let name = path
        .name_lossy()
        .map(|n| n.into_owned())
        .unwrap_or_else(|| "download".into());
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
    let disposition = if attachment { "attachment" } else { "inline" };
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

async fn save_text(
    axum::extract::State(s): axum::extract::State<AppState>,
    fu: FileUser,
    Path(raw_path): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Body,
) -> Result<axum::response::Response, ApiError> {
    use axum::http::header;
    use axum::response::IntoResponse;
    use futures_util::StreamExt;

    use crate::fs::edit::Expect;

    fu.need(false, true)?;
    let path = RelPath::parse(&raw_path)?;
    let text_header = |h| {
        headers
            .get(h)
            .and_then(|v: &axum::http::HeaderValue| v.to_str().ok())
            .map(str::trim)
    };
    let expect = match (
        text_header(header::IF_MATCH),
        text_header(header::IF_NONE_MATCH),
    ) {
        (Some(tag), _) => Expect::Etag(tag.to_string()),
        (None, Some("*")) => Expect::New,
        _ => {
            return Err(ApiError::new(
                StatusCode::PRECONDITION_REQUIRED,
                "if_match_required",
                "Reload the file before saving it.",
            ));
        }
    };
    let max = s.config.edit_max;
    let mut text = Vec::new();
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|_| ApiError::bad_request("The request body could not be read."))?;
        if (text.len() + chunk.len()) as u64 > max {
            return Err(FsError::TooLarge(max).into());
        }
        text.extend_from_slice(&chunk);
    }
    let st = crate::fs::edit::save_text(&fu.scope, s.db.clone(), &path, text, expect, crate::now())
        .await?;
    let tag = crate::fs::raw::etag(&st);
    tracing::info!(event = "file.save", user = fu.user.id, path = %path);
    Ok(([(header::ETAG, tag.clone())], Json(json!({ "etag": tag }))).into_response())
}

async fn zip_root(fu: FileUser) -> Result<axum::response::Response, ApiError> {
    zip_at(fu, String::new()).await
}

async fn zip_path(
    fu: FileUser,
    Path(raw_path): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    zip_at(fu, raw_path).await
}

async fn zip_at(fu: FileUser, raw_path: String) -> Result<axum::response::Response, ApiError> {
    zip_response(fu, raw_path, None).await
}

async fn zip_root_items(
    fu: FileUser,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, ApiError> {
    zip_response(fu, String::new(), Some(zip_items(&body)?)).await
}

async fn zip_path_items(
    fu: FileUser,
    Path(raw_path): Path<String>,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, ApiError> {
    zip_response(fu, raw_path, Some(zip_items(&body)?)).await
}

/// Most items one ZIP of chosen items takes.
const ZIP_ITEMS_MAX: usize = 1000;

/// The `item` values of a form body: each one name inside the folder, no repeats, 1 to 1000 of them.
fn zip_items(body: &[u8]) -> Result<Vec<Vec<u8>>, ApiError> {
    let bad = || ApiError::bad_request("Choose between 1 and 1000 items in one folder.");
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for (key, value) in form_urlencoded::parse(body) {
        if key != "item" || value.contains('/') {
            return Err(bad());
        }
        let one = RelPath::parse(&value).map_err(|_| bad())?;
        let [name] = one.segments() else {
            return Err(bad());
        };
        if !seen.insert(name.clone()) || items.len() == ZIP_ITEMS_MAX {
            return Err(bad());
        }
        items.push(name.clone());
    }
    if items.is_empty() {
        return Err(bad());
    }
    Ok(items)
}

async fn zip_response(
    fu: FileUser,
    raw_path: String,
    items: Option<Vec<Vec<u8>>>,
) -> Result<axum::response::Response, ApiError> {
    fu.need(true, false)?;
    let path = RelPath::parse(&raw_path)?;
    serve_zip(fu.scope, path, items, "files").await
}

/// A folder, or chosen items in it, as a streamed ZIP. `root_name` names the archive of the scope's own folder.
pub(crate) async fn serve_zip(
    scope: Scope,
    path: RelPath,
    items: Option<Vec<Vec<u8>>>,
    root_name: &str,
) -> Result<axum::response::Response, ApiError> {
    use axum::http::header;
    use axum::response::IntoResponse;

    let name = match path.name_lossy() {
        Some(n) => format!("{n}.zip"),
        None => format!("{root_name}.zip"),
    };
    // Checked before the response starts, so a wrong path or a missing item is an ordinary JSON error.
    let (scope, path, items) = blocking(move || {
        scope.open_dir(&path)?;
        let items = match items {
            None => None,
            Some(names) => Some(
                names
                    .iter()
                    .map(|n| {
                        let item = path.join_bytes(n)?;
                        scope.stat(&item)?;
                        Ok(item)
                    })
                    .collect::<Result<Vec<_>, FsError>>()?,
            ),
        };
        Ok((scope, path, items))
    })
    .await?;
    let headers = [
        (header::CONTENT_TYPE, "application/zip".to_string()),
        (
            header::CONTENT_DISPOSITION,
            format!(
                "attachment; filename*=UTF-8''{}",
                crate::fs::raw::encode_filename(&name)
            ),
        ),
        (header::CONTENT_SECURITY_POLICY, RAW_CSP.to_string()),
    ];
    Ok((headers, crate::fs::zip::stream_zip(scope, path, items)).into_response())
}
