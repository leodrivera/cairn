//! Security headers on every response.

use axum::Router;
use axum::http::{HeaderName, HeaderValue, header};
use tower_http::set_header::SetResponseHeaderLayer;

const CSP: &str = "default-src 'self'; img-src 'self' blob: data:; style-src 'self'; script-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

/// The app CSP goes only where a route set none: file and share routes send a stricter one of their own.
pub fn apply(router: Router) -> Router {
    let set = |name: HeaderName, value: &'static str| {
        SetResponseHeaderLayer::overriding(name, HeaderValue::from_static(value))
    };
    router
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        ))
        .layer(set(header::REFERRER_POLICY, "no-referrer"))
        .layer(set(
            HeaderName::from_static("cross-origin-opener-policy"),
            "same-origin",
        ))
        .layer(set(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn route_csp_is_kept() {
        let strict = "default-src 'none'; sandbox";
        let app = apply(Router::new().route(
            "/raw",
            get(move || async move { ([(header::CONTENT_SECURITY_POLICY, strict)], "x") }),
        ));
        let res = app
            .oneshot(Request::get("/raw").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.headers()[header::CONTENT_SECURITY_POLICY], strict);
    }
}
