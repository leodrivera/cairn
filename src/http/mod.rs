//! HTTP: the router every route hangs from.

pub mod admin_routes;
pub mod assets;
pub mod auth_routes;
pub mod client;
pub mod csrf;
pub mod current_user;
pub mod error;
pub mod files_routes;
pub mod headers;
pub mod json;
pub mod users_routes;

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router, middleware};
use serde_json::json;

use crate::auth::captcha::Captcha;
use crate::auth::password::Hasher;
use crate::auth::ratelimit::Limiters;
use crate::config::Config;
use crate::db::Db;

/// Shared by every handler.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Arc<Db>,
    pub hasher: Arc<Hasher>,
    pub limiters: Arc<Limiters>,
    pub captcha: Option<Arc<Captcha>>,
    /// Device and inode of `CAIRN_DATA`: no path a client sends may pass through it.
    pub data_id: Option<(u64, u64)>,
    /// Tests only: resolve paths the way kernels without openat2 do.
    pub force_fallback: bool,
}

impl AppState {
    pub fn new(config: Config, db: Db) -> AppState {
        let captcha =
            Captcha::from_config(&config.captcha, config.public_url.as_ref()).map(Arc::new);
        let data_id = std::fs::metadata(&config.data).ok().map(|m| {
            use std::os::unix::fs::MetadataExt;
            (m.dev(), m.ino())
        });
        AppState {
            data_id,
            force_fallback: false,
            config: Arc::new(config),
            db: Arc::new(db),
            hasher: Arc::new(Hasher::new()),
            limiters: Arc::new(Limiters::new()),
            captcha,
        }
    }
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .merge(auth_routes::routes())
        .merge(admin_routes::routes())
        .merge(users_routes::routes())
        .merge(files_routes::routes())
        .fallback(api_not_found)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            current_user::load,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            csrf::require_header,
        ))
        .layer(DefaultBodyLimit::max(1 << 20));
    let captcha = state.captcha.clone();
    let app = Router::new()
        .nest("/api", api)
        .fallback(assets::serve)
        .with_state(state);
    headers::apply(app, captcha.as_deref())
}

async fn health() -> impl IntoResponse {
    Json(json!({ "ok": true }))
}

async fn api_not_found() -> impl IntoResponse {
    error::ApiError::not_found()
}

#[cfg(test)]
mod tests {

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;
    use crate::config::{Config, Vars};

    fn test_state() -> AppState {
        AppState::new(
            Config::from_vars(&Vars::new()).unwrap(),
            crate::db::Db::open_in_memory(),
        )
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

    async fn send(req: Request<Body>) -> axum::response::Response {
        router(test_state()).oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn serves_brotli() {
        let res = send(
            Request::get("/")
                .header("accept-encoding", "gzip, br")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(header(&res, "content-encoding"), "br");
        assert_eq!(header(&res, "vary"), "accept-encoding");
        assert!(header(&res, "content-type").starts_with("text/html"));
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let want = std::fs::read("web/dist/index.html.br").unwrap();
        assert_eq!(body.as_ref(), want.as_slice());
    }

    #[tokio::test]
    async fn serves_gzip() {
        let res = send(
            Request::get("/")
                .header("accept-encoding", "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(header(&res, "content-encoding"), "gzip");
    }

    #[tokio::test]
    async fn serves_identity_without_header() {
        let res = get("/").await;
        assert!(res.headers().get("content-encoding").is_none());
        assert_eq!(header(&res, "vary"), "accept-encoding");
        assert!(body_text(res).await.starts_with("<!doctype html>"));
    }

    #[tokio::test]
    async fn post_to_page_is_405() {
        let res = send(Request::post("/files/a").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(header(&res, "allow"), "GET, HEAD");
    }

    #[tokio::test]
    async fn head_works() {
        let res = send(Request::head("/").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(header(&res, "content-type").starts_with("text/html"));
    }

    #[tokio::test]
    async fn compressed_path_not_exposed() {
        let res = send(Request::get("/index.html.br").body(Body::empty()).unwrap()).await;
        assert!(header(&res, "content-type").starts_with("text/html"));
        assert!(res.headers().get("content-encoding").is_none());
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
