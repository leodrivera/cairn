//! Security headers on every response.

use axum::Router;
use axum::http::{HeaderName, HeaderValue, header};
use tower_http::set_header::SetResponseHeaderLayer;

const CSP: &str = "default-src 'self'; img-src 'self' blob: data:; style-src 'self'; script-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

pub fn apply(router: Router) -> Router {
    let set = |name: HeaderName, value: &'static str| {
        SetResponseHeaderLayer::overriding(name, HeaderValue::from_static(value))
    };
    router
        .layer(set(header::CONTENT_SECURITY_POLICY, CSP))
        .layer(set(header::REFERRER_POLICY, "no-referrer"))
        .layer(set(
            HeaderName::from_static("cross-origin-opener-policy"),
            "same-origin",
        ))
        .layer(set(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
}
