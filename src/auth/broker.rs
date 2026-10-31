//! The OAuth broker behind the provider buttons (Tinyauth v5). Two of its unauthenticated endpoints:
//! `GET /api/context/app` lists the configured providers; `GET /api/oauth/url/{id}` opens a pending OAuth session,
//! answers the provider's authorize URL and sets the cookie its callback needs. The browser cannot call the latter
//! itself (no CORS), so Cairn does and relays the cookies.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::HeaderValue;
use serde::Deserialize;
use url::Url;

use crate::config::OauthBroker;

/// A good list stays fresh this long; after a failure the next try comes sooner.
const FRESH_SECS: i64 = 60;
const RETRY_SECS: i64 = 20;
const BODY_LIMIT: u64 = 64 * 1024;
const MAX_COOKIES: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct BrokerProvider {
    pub id: String,
    pub name: String,
}

pub struct Broker {
    base: String,
    list_agent: ureq::Agent,
    start_agent: ureq::Agent,
    cache: Mutex<(Vec<BrokerProvider>, i64)>,
    refreshing: AtomicBool,
}

#[derive(Deserialize)]
struct App {
    auth: AppAuth,
}

#[derive(Deserialize)]
struct AppAuth {
    #[serde(default)]
    providers: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct Started {
    url: String,
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .tls_config(crate::tls::config())
        .build()
        .into()
}

impl Broker {
    pub fn new(c: &OauthBroker) -> Broker {
        Broker {
            base: c.url.as_str().trim_end_matches('/').to_string(),
            list_agent: agent(Duration::from_secs(2)),
            start_agent: agent(Duration::from_secs(5)),
            cache: Mutex::new((Vec::new(), 0)),
            refreshing: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (Vec<BrokerProvider>, i64)> {
        self.cache.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Fetches the provider list once. A refresh already running makes this return at once.
    pub async fn refresh(&self) {
        if self.refreshing.swap(true, Ordering::AcqRel) {
            return;
        }
        let (agent, url) = (
            self.list_agent.clone(),
            format!("{}/api/context/app", self.base),
        );
        let got = tokio::task::spawn_blocking(move || -> Result<Vec<BrokerProvider>, String> {
            let mut res = agent.get(&url).call().map_err(|e| e.to_string())?;
            if !res.status().is_success() {
                return Err(format!("HTTP {}", res.status().as_u16()));
            }
            let app: App = res
                .body_mut()
                .with_config()
                .limit(BODY_LIMIT)
                .read_json()
                .map_err(|e| e.to_string())?;
            Ok(app
                .auth
                .providers
                .iter()
                .filter(|p| p["oauth"] == serde_json::Value::Bool(true))
                .filter_map(|p| {
                    let id = p["id"].as_str().filter(|id| !id.is_empty())?;
                    let name = p["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(id);
                    Some(BrokerProvider {
                        id: id.to_string(),
                        name: name.to_string(),
                    })
                })
                .collect())
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        let now = crate::now();
        {
            let mut cache = self.lock();
            match got {
                Ok(list) => *cache = (list, now + FRESH_SECS),
                Err(error) => {
                    // The endpoints are those of Tinyauth v5; another major version may move them, which hides the buttons.
                    tracing::warn!(
                        event = "oauth.broker_unreachable",
                        error = %error,
                        hint = "Cairn calls the Tinyauth v5 API (/api/context/app); check the broker is up and on v5",
                    );
                    cache.1 = now + RETRY_SECS;
                }
            }
        }
        self.refreshing.store(false, Ordering::Release);
    }

    /// Tests only: makes the cached list due for a refresh now, keeping it. Nothing in the environment reaches this.
    #[doc(hidden)]
    pub fn mark_stale(&self) {
        self.lock().1 = 0;
    }

    /// The last good list, and whether it is due for a refresh.
    pub fn cached(&self, now: i64) -> (Vec<BrokerProvider>, bool) {
        let cache = self.lock();
        (cache.0.clone(), now >= cache.1)
    }

    /// The list at once; a stale one is refreshed in the background, so a slow broker never delays a page.
    pub fn providers(self: &Arc<Self>) -> Vec<BrokerProvider> {
        let (list, stale) = self.cached(crate::now());
        if stale && !self.refreshing.load(Ordering::Acquire) {
            let me = self.clone();
            tokio::spawn(async move { me.refresh().await });
        }
        list
    }

    /// Opens a pending OAuth session: the provider's authorize URL (always https) and the cookies to relay.
    pub async fn start(
        &self,
        id: &str,
        redirect_uri: &str,
    ) -> Result<(Url, Vec<HeaderValue>), String> {
        let mut url =
            Url::parse(&format!("{}/api/oauth/url/", self.base)).map_err(|e| e.to_string())?;
        url.path_segments_mut()
            .map_err(|_| "broker URL cannot take a path".to_string())?
            .pop_if_empty()
            .push(id);
        url.query_pairs_mut()
            .append_pair("redirect_uri", redirect_uri);
        let agent = self.start_agent.clone();
        tokio::task::spawn_blocking(move || {
            let mut res = agent.get(url.as_str()).call().map_err(|e| e.to_string())?;
            if !res.status().is_success() {
                return Err(format!("HTTP {}", res.status().as_u16()));
            }
            let cookies: Vec<HeaderValue> = res
                .headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|v| HeaderValue::from_bytes(v.as_bytes()).ok())
                .take(MAX_COOKIES)
                .collect();
            let started: Started = res
                .body_mut()
                .with_config()
                .limit(BODY_LIMIT)
                .read_json()
                .map_err(|e| e.to_string())?;
            let target = Url::parse(&started.url).map_err(|e| e.to_string())?;
            if target.scheme() != "https" {
                return Err("the authorize URL is not https".into());
            }
            Ok((target, cookies))
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use axum::extract::{Path, RawQuery};
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};
    use axum::routing::get;
    use serde_json::{Value, json};

    use super::*;
    use crate::config::{Config, Vars};

    #[derive(Default)]
    struct Fake {
        list: Value,
        list_status: u16,
        start_url: String,
        start_cookies: usize,
        delay: Duration,
        seen: Vec<String>,
    }

    async fn fake(f: Fake) -> (String, Arc<Mutex<Fake>>) {
        let state = Arc::new(Mutex::new(f));
        let (a, b) = (state.clone(), state.clone());
        let app =
            axum::Router::new()
                .route(
                    "/api/context/app",
                    get(move || async move {
                        let (status, body, delay) = {
                            let f = a.lock().unwrap();
                            (f.list_status, f.list.to_string(), f.delay)
                        };
                        tokio::time::sleep(delay).await;
                        let status =
                            StatusCode::from_u16(if status == 0 { 200 } else { status }).unwrap();
                        (status, [("content-type", "application/json")], body)
                    }),
                )
                .route(
                    "/api/oauth/url/{id}",
                    get(
                        move |Path(id): Path<String>, RawQuery(q): RawQuery| async move {
                            let (url, cookies, delay) = {
                                let mut f = b.lock().unwrap();
                                f.seen.push(format!("{id}?{}", q.unwrap_or_default()));
                                (f.start_url.clone(), f.start_cookies, f.delay)
                            };
                            tokio::time::sleep(delay).await;
                            let mut res: Response = (
                                [("content-type", "application/json")],
                                json!({ "url": url }).to_string(),
                            )
                                .into_response();
                            for i in 0..cookies {
                                res.headers_mut().append(
                            "set-cookie",
                            format!("tinyauth-csrf-{i}=x{i}; Path=/; Domain=example.com; HttpOnly")
                                .parse()
                                .unwrap(),
                        );
                            }
                            res
                        },
                    ),
                );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, state)
    }

    fn broker(url: &str) -> Arc<Broker> {
        let vars: Vars = [
            ("CAIRN_AUTH_METHODS", "oauth"),
            ("CAIRN_PUBLIC_URL", "https://files.example.com"),
            ("CAIRN_OAUTH_BROKER_URL", url),
            ("CAIRN_OAUTH_SECRET", "0123456789abcdef0123456789abcdef"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let c = Config::from_vars(&vars).unwrap();
        Arc::new(Broker::new(c.auth.oauth.as_ref().unwrap()))
    }

    fn list() -> Value {
        json!({"auth":{"providers":[
            {"id":"google","name":"Google","oauth":true},
            {"id":"local","name":"Local","oauth":false},
            {"id":"","oauth":true},
            {"id":"github","oauth":true}
        ]}})
    }

    fn p(id: &str, name: &str) -> BrokerProvider {
        BrokerProvider {
            id: id.into(),
            name: name.into(),
        }
    }

    #[tokio::test]
    async fn parses_provider_list() {
        let (url, _) = fake(Fake {
            list: list(),
            ..Default::default()
        })
        .await;
        let b = broker(&url);
        b.refresh().await;
        assert_eq!(
            b.cached(crate::now()).0,
            vec![p("google", "Google"), p("github", "github")]
        );
    }

    #[tokio::test]
    async fn fresh_for_60s() {
        let (url, _) = fake(Fake {
            list: list(),
            ..Default::default()
        })
        .await;
        let b = broker(&url);
        let now = crate::now();
        b.refresh().await;
        assert!(!b.cached(now + 59).1);
        assert!(b.cached(now + 61).1);
    }

    #[tokio::test]
    async fn failure_keeps_last_list() {
        let (url, f) = fake(Fake {
            list: list(),
            ..Default::default()
        })
        .await;
        let b = broker(&url);
        b.refresh().await;
        f.lock().unwrap().list_status = 500;
        let now = crate::now();
        b.refresh().await;
        let (got, stale) = b.cached(now);
        assert_eq!(got.len(), 2);
        assert!(!stale);
        assert!(!b.cached(now + 19).1);
        assert!(b.cached(now + 21).1);
    }

    #[tokio::test]
    async fn failure_first_is_empty() {
        let (url, _) = fake(Fake {
            list_status: 500,
            ..Default::default()
        })
        .await;
        let b = broker(&url);
        b.refresh().await;
        assert!(b.cached(crate::now()).0.is_empty());
    }

    #[tokio::test]
    async fn start_returns_url_and_cookies() {
        let (url, f) = fake(Fake {
            start_url: "https://provider.example.com/authorize?x=1".into(),
            start_cookies: 2,
            ..Default::default()
        })
        .await;
        let (target, cookies) = broker(&url)
            .start("google", "https://files.example.com/auth/sso")
            .await
            .unwrap();
        assert_eq!(
            target.as_str(),
            "https://provider.example.com/authorize?x=1"
        );
        assert_eq!(cookies.len(), 2);
        assert!(
            cookies[0]
                .to_str()
                .unwrap()
                .starts_with("tinyauth-csrf-0=x0")
        );
        assert_eq!(
            f.lock().unwrap().seen,
            vec!["google?redirect_uri=https%3A%2F%2Ffiles.example.com%2Fauth%2Fsso".to_string()]
        );
    }

    #[tokio::test]
    async fn start_refuses_http_url() {
        let (url, _) = fake(Fake {
            start_url: "http://provider.example.com/authorize".into(),
            ..Default::default()
        })
        .await;
        assert!(
            broker(&url)
                .start("google", "https://files.example.com/auth/sso")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn start_caps_cookies() {
        let (url, _) = fake(Fake {
            start_url: "https://provider.example.com/authorize".into(),
            start_cookies: 12,
            ..Default::default()
        })
        .await;
        let (_, cookies) = broker(&url)
            .start("google", "https://files.example.com/auth/sso")
            .await
            .unwrap();
        assert_eq!(cookies.len(), 8);
    }

    #[tokio::test]
    async fn start_timeout() {
        let (url, _) = fake(Fake {
            start_url: "https://provider.example.com/authorize".into(),
            delay: Duration::from_secs(6),
            ..Default::default()
        })
        .await;
        let t = Instant::now();
        assert!(
            broker(&url)
                .start("google", "https://files.example.com/auth/sso")
                .await
                .is_err()
        );
        assert!(
            t.elapsed() < Duration::from_millis(5500),
            "{:?}",
            t.elapsed()
        );
    }
}
