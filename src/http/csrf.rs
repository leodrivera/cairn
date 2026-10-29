//! CSRF: every state-changing request carries `X-Cairn`. Browsers send no custom header cross-site without a CORS
//! preflight, which Cairn never allows; with `SameSite=Lax` cookies that is enough.
//!
//! One exception: `POST /api/zip/...` with a list of chosen items comes from a plain HTML form, so the browser downloads
//! the archive itself, and a form cannot send a custom header. That route changes nothing; it needs an `Origin` naming
//! this host instead, which browsers always send on form posts and a page on another site cannot fake.

use axum::extract::Request;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::http::error::ApiError;

/// Whether `Origin` names the host the request was sent to (scheme aside).
fn same_origin(headers: &HeaderMap) -> bool {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let authority = origin.and_then(|o| {
        o.strip_prefix("https://")
            .or_else(|| o.strip_prefix("http://"))
    });
    matches!((authority, host), (Some(a), Some(h)) if !a.is_empty() && a.eq_ignore_ascii_case(h))
}

pub async fn require_header(req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    let path = req.uri().path();
    let zip_form = *req.method() == Method::POST && (path == "/zip" || path.starts_with("/zip/"));
    let ok = safe
        || if zip_form {
            same_origin(req.headers())
        } else {
            req.headers().contains_key("x-cairn")
        };
    if !ok {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            "csrf",
            "Missing X-Cairn header. Reload the page and try again.",
        )
        .into_response();
    }
    next.run(req).await
}
