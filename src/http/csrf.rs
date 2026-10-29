//! CSRF: every state-changing request carries `X-Cairn`. Browsers send no custom header cross-site without a CORS
//! preflight, which Cairn never allows; with `SameSite=Lax` cookies that is enough.

use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::http::error::ApiError;

pub async fn require_header(req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && !req.headers().contains_key("x-cairn") {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            "csrf",
            "Missing X-Cairn header. Reload the page and try again.",
        )
        .into_response();
    }
    next.run(req).await
}
