//! HTTP: the router every route hangs from.

pub mod assets;
pub mod headers;

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use crate::config::Config;

/// Shared by every handler. Later parts add the database and the rate limiters here.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .fallback(api_not_found);
    let app = Router::new()
        .nest("/api", api)
        .fallback(assets::serve)
        .with_state(state);
    headers::apply(app)
}

async fn health() -> impl IntoResponse {
    Json(json!({ "ok": true }))
}

async fn api_not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "not_found", "message": "No such API route." })),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;
    use crate::config::{Config, Vars};

    fn test_state() -> AppState {
        AppState {
            config: Arc::new(Config::from_vars(&Vars::new()).unwrap()),
        }
    }

    async fn get(path: &str) -> axum::response::Response {
        router(test_state())
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn body_text(res: axum::response::Response) -> String {
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
    }

    fn header<'a>(res: &'a axum::response::Response, name: &str) -> &'a str {
        res.headers()
            .get(name)
            .unwrap_or_else(|| panic!("missing {name}"))
            .to_str()
            .unwrap()
    }

    #[tokio::test]
    async fn health_ok() {
        let res = get("/api/health").await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(header(&res, "content-type"), "application/json");
        assert_eq!(body_text(res).await, r#"{"ok":true}"#);
    }

    #[tokio::test]
    async fn unknown_api_is_json_404() {
        let res = get("/api/nope").await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            body_text(res).await,
            r#"{"error":"not_found","message":"No such API route."}"#
        );
    }

    #[tokio::test]
    async fn spa_fallback() {
        let res = get("/some/folder").await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(header(&res, "content-type").starts_with("text/html"));
        assert_eq!(header(&res, "cache-control"), "no-cache");
    }

    #[tokio::test]
    async fn security_headers_everywhere() {
        for path in ["/", "/api/health", "/api/nope"] {
            let res = get(path).await;
            assert_eq!(
                header(&res, "content-security-policy"),
                "default-src 'self'; img-src 'self' blob: data:; style-src 'self'; script-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
                "{path}"
            );
            assert_eq!(header(&res, "referrer-policy"), "no-referrer", "{path}");
            assert_eq!(
                header(&res, "cross-origin-opener-policy"),
                "same-origin",
                "{path}"
            );
            assert_eq!(header(&res, "x-content-type-options"), "nosniff", "{path}");
        }
    }
}
