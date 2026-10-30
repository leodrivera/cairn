//! A fake OAuth broker with the two Tinyauth v5 endpoints Cairn calls.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};

#[derive(Default)]
struct State {
    list: Value,
    list_status: u16,
    start_url: String,
    start_status: u16,
    delay: Duration,
    redirect_uris: Vec<String>,
    hits: usize,
}

pub struct FakeBroker {
    pub url: String,
    state: Arc<Mutex<State>>,
}

impl FakeBroker {
    /// `providers` is the `auth.providers` array the broker lists.
    pub async fn start(providers: Value) -> FakeBroker {
        let state = Arc::new(Mutex::new(State {
            list: json!({ "auth": { "providers": providers } }),
            start_url: "https://provider.example.com/authorize?state=abc".into(),
            ..Default::default()
        }));
        let (a, b) = (state.clone(), state.clone());
        let app = axum::Router::new()
            .route(
                "/api/context/app",
                get(move || async move {
                    let (status, body, delay) = {
                        let mut s = a.lock().unwrap();
                        s.hits += 1;
                        (s.list_status, s.list.to_string(), s.delay)
                    };
                    tokio::time::sleep(delay).await;
                    let status = StatusCode::from_u16(if status == 0 { 200 } else { status }).unwrap();
                    (status, [("content-type", "application/json")], body).into_response()
                }),
            )
            .route(
                "/api/oauth/url/{id}",
                get(
                    move |Path(_id): Path<String>,
                          Query(q): Query<std::collections::HashMap<String, String>>| async move {
                        let (url, status) = {
                            let mut s = b.lock().unwrap();
                            s.hits += 1;
                            s.redirect_uris
                                .push(q.get("redirect_uri").cloned().unwrap_or_default());
                            (s.start_url.clone(), s.start_status)
                        };
                        if status != 0 {
                            return StatusCode::from_u16(status).unwrap().into_response();
                        }
                        let mut res: Response = (
                            [("content-type", "application/json")],
                            json!({ "url": url }).to_string(),
                        )
                            .into_response();
                        for c in [
                            "tinyauth-csrf=one; Path=/; Domain=example.com; HttpOnly",
                            "tinyauth-redirect=two; Path=/; Domain=example.com; HttpOnly",
                        ] {
                            res.headers_mut().append("set-cookie", c.parse().unwrap());
                        }
                        res
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeBroker { url, state }
    }

    pub fn set_delay(&self, d: Duration) {
        self.state.lock().unwrap().delay = d;
    }
    pub fn set_list_status(&self, status: u16) {
        self.state.lock().unwrap().list_status = status;
    }
    pub fn set_start_status(&self, status: u16) {
        self.state.lock().unwrap().start_status = status;
    }
    pub fn set_start_url(&self, url: &str) {
        self.state.lock().unwrap().start_url = url.into();
    }
    pub fn last_redirect_uri(&self) -> Option<String> {
        self.state.lock().unwrap().redirect_uris.last().cloned()
    }
    pub fn hits(&self) -> usize {
        self.state.lock().unwrap().hits
    }
}
