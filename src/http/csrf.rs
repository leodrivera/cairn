//! CSRF: every state-changing request carries `X-Cairn`. Browsers send no custom header cross-site without a CORS
//! preflight, which Cairn never allows; with `SameSite=Lax` cookies that is enough.
//!
//! One exception: `POST /api/zip/...` with a list of chosen items comes from a plain HTML form, so the browser downloads
//! the archive itself, and a form cannot send a custom header. That route changes nothing; it needs an `Origin` naming
//! this site instead: `Sec-Fetch-Site: same-origin` when the browser sends it, else an `Origin` whose host is the one in
//! `CAIRN_PUBLIC_URL` or in `Host`. A page on another site can fake neither. The public address matters behind proxies
//! that send their upstream name as `Host` (nginx does by default).

use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::http::AppState;
use crate::http::error::ApiError;

/// Whether a form post came from this site (see the module comment).
fn same_site(headers: &HeaderMap, public: Option<&str>) -> bool {
    if let Some(site) = headers.get("sec-fetch-site") {
        return site.as_bytes() == b"same-origin";
    }
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let authority = origin.and_then(|o| {
        o.strip_prefix("https://")
            .or_else(|| o.strip_prefix("http://"))
    });
    let Some(authority) = authority.filter(|a| !a.is_empty()) else {
        return false;
    };
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    [public, host]
        .into_iter()
        .flatten()
        .any(|h| authority.eq_ignore_ascii_case(h))
}

pub async fn require_header(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    let path = req.uri().path();
    let zip_form = *req.method() == Method::POST && (path == "/zip" || path.starts_with("/zip/"));
    let ok = safe
        || if zip_form {
            let public = s.config.public_url.as_ref().map(|u| {
                let host = u.host_str().unwrap_or("");
                match u.port() {
                    Some(p) => format!("{host}:{p}"),
                    None => host.to_string(),
                }
            });
            same_site(req.headers(), public.as_deref())
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
