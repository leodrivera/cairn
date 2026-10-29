//! In-process test harness: the real router over an in-memory database.
#![allow(dead_code)]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use cairn::auth::captcha::Captcha;
use cairn::auth::password::hash_blocking;
use cairn::config::{Config, Vars};
use cairn::db::Db;
use cairn::http::client::Peer;
use cairn::http::{AppState, router};
use cairn::users::{self, NewUser, Perms, User};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

pub struct Harness {
    pub state: AppState,
    pub app: Router,
}

pub fn harness(extra: &[(&str, &str)]) -> Harness {
    let mut vars: Vars = Vars::new();
    for (k, v) in extra {
        vars.insert(k.to_string(), v.to_string());
    }
    let config = Config::from_vars(&vars).unwrap();
    let state = AppState::new(config, Db::open_in_memory());
    Harness {
        app: router(state.clone()),
        state,
    }
}

/// A server root and a data folder on disk, for the file routes. Keep the `TempDir` alive for the test.
pub fn harness_with_root(extra: &[(&str, &str)]) -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let (srv, data) = (dir.path().join("srv"), dir.path().join("data"));
    std::fs::create_dir_all(&srv).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let (srv, data) = (
        srv.to_str().unwrap().to_string(),
        data.to_str().unwrap().to_string(),
    );
    let mut all = vec![("CAIRN_ROOT", srv.as_str()), ("CAIRN_DATA", data.as_str())];
    all.extend_from_slice(extra);
    (harness(&all), dir)
}

/// Percent-encodes a path for a URL, keeping the slashes.
pub fn enc(path: &str) -> String {
    path.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl Harness {
    pub fn add_user_scoped(&self, email: &str, password: &str, scope: &str, perms: Perms) -> User {
        let new = NewUser {
            email: email.into(),
            name: "Someone".into(),
            password_hash: Some(hash_blocking(password)),
            scope: scope.into(),
            perms,
        };
        users::create(&self.state.db, new, 0).unwrap()
    }

    /// Points the CAPTCHA client at a fake vendor.
    pub fn with_captcha_url(mut self, url: String) -> Harness {
        let c = &self.state.config;
        let captcha = Captcha::from_config(&c.captcha, c.public_url.as_ref())
            .unwrap()
            .with_verify_url(url);
        self.state.captcha = Some(Arc::new(captcha));
        self.app = router(self.state.clone());
        self
    }

    pub fn add_user(&self, email: &str, password: Option<&str>, admin: bool) -> User {
        let perms = Perms {
            read: true,
            write: admin,
            share: admin,
            admin,
        };
        let new = NewUser {
            email: email.into(),
            name: "Someone".into(),
            password_hash: password.map(hash_blocking),
            scope: "/".into(),
            perms,
        };
        users::create(&self.state.db, new, 0).unwrap()
    }

    pub async fn login(&self, email: &str, password: &str) -> String {
        let res = Req::post("/api/auth/login")
            .json(serde_json::json!({ "email": email, "password": password, "keep": false }))
            .send(&self.app)
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.json);
        res.cookie().expect("session cookie")
    }
}

pub struct Req {
    method: Method,
    path: String,
    body: Option<Value>,
    headers: Vec<(String, String)>,
    csrf: bool,
    peer: Option<std::net::SocketAddr>,
    raw: Option<Vec<u8>>,
}

pub struct Res {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub json: Value,
    pub body: Vec<u8>,
}

impl Res {
    /// The `name=value` pair from the session cookie the response set.
    pub fn cookie(&self) -> Option<String> {
        self.set_cookie()
            .map(|c| c.split(';').next().unwrap().to_string())
    }

    pub fn set_cookie(&self) -> Option<String> {
        self.headers
            .get("set-cookie")
            .map(|v| v.to_str().unwrap().to_string())
    }

    pub fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .map(|v| v.to_str().unwrap())
            .unwrap_or("")
    }

    pub fn code(&self) -> &str {
        self.json["error"].as_str().unwrap_or("")
    }
}

impl Req {
    fn new(method: Method, path: &str) -> Req {
        Req {
            method,
            path: path.into(),
            body: None,
            headers: Vec::new(),
            csrf: true,
            peer: None,
            raw: None,
        }
    }
    pub fn get(path: &str) -> Req {
        Req::new(Method::GET, path)
    }
    pub fn post(path: &str) -> Req {
        Req::new(Method::POST, path)
    }
    pub fn put(path: &str) -> Req {
        Req::new(Method::PUT, path)
    }
    pub fn patch(path: &str) -> Req {
        Req::new(Method::PATCH, path)
    }
    pub fn delete(path: &str) -> Req {
        Req::new(Method::DELETE, path)
    }
    /// A raw body, like a file upload or a text save.
    pub fn bytes(mut self, b: &[u8]) -> Req {
        self.raw = Some(b.to_vec());
        self
    }
    pub fn json(mut self, v: Value) -> Req {
        self.body = Some(v);
        self
    }
    pub fn raw_json(mut self, text: &str) -> Req {
        self.body = Some(Value::String(format!("\u{0}RAW{text}")));
        self
    }
    pub fn cookie(self, c: &str) -> Req {
        self.header("cookie", c)
    }
    pub fn header(mut self, k: &str, v: &str) -> Req {
        self.headers.push((k.into(), v.into()));
        self
    }
    pub fn no_csrf(mut self) -> Req {
        self.csrf = false;
        self
    }
    pub fn peer(mut self, addr: &str) -> Req {
        self.peer = Some(addr.parse().unwrap());
        self
    }

    pub async fn send(self, app: &Router) -> Res {
        let mut b = Request::builder().method(self.method).uri(&self.path);
        if self.csrf {
            b = b.header("x-cairn", "1");
        }
        for (k, v) in &self.headers {
            b = b.header(k, v);
        }
        let body = match (self.raw, self.body) {
            (Some(raw), _) => Body::from(raw),
            (None, body) => match body {
                Some(Value::String(s)) if s.starts_with("\u{0}RAW") => {
                    b = b.header("content-type", "application/json");
                    Body::from(s.trim_start_matches("\u{0}RAW").to_string())
                }
                Some(v) => {
                    b = b.header("content-type", "application/json");
                    Body::from(v.to_string())
                }
                None => Body::empty(),
            },
        };
        let mut req = b.body(body).unwrap();
        if let Some(p) = self.peer {
            req.extensions_mut().insert(Peer(p));
        }
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Res {
            status,
            headers,
            json,
            body: bytes.to_vec(),
        }
    }
}
