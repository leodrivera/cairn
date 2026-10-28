//! The front-end, embedded in the binary at build time.

use axum::http::{HeaderValue, Uri, header};
use axum::response::{IntoResponse, Response};

#[derive(rust_embed::Embed)]
#[folder = "web/dist/"]
struct Dist;

/// A file from the front-end build, or `index.html` for any other path (the front-end routes those).
pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let (path, file) = match Dist::get(path).filter(|_| !path.is_empty()) {
        Some(file) => (path, file),
        None => match Dist::get("index.html") {
            Some(file) => ("index.html", file),
            None => {
                return (
                    axum::http::StatusCode::NOT_FOUND,
                    "The front-end is missing from this build.",
                )
                    .into_response();
            }
        },
    };
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(mime(path))),
            (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
        ],
        file.data,
    )
        .into_response()
}

fn mime(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("png") => "image/png",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        _ => "application/octet-stream",
    }
}
