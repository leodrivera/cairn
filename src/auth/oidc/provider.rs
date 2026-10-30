//! OIDC providers: the discovery document and JWKS of each, fetched at startup and retried in the background.

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::auth::oidc::jwt::{self, Expect, Identity, Jwks, TokenError};
use crate::config::{OidcProvider, Secret};

const TIMEOUT: Duration = Duration::from_secs(5);
const BODY_LIMIT: u64 = 64 * 1024;
/// A provider that could not be reached is tried again this often.
const RETRY_SECS: u64 = 60;
/// An unknown `kid` refetches the JWKS at most this often.
const JWKS_REFRESH_SECS: i64 = 60;

/// What discovery gave: where to send people, where to trade the code, and the keys.
#[derive(Clone)]
pub(crate) struct Ready {
    pub authorization: Url,
    pub token: Url,
    pub jwks_uri: Url,
    /// The token endpoint takes the client secret as HTTP Basic, not in the body.
    pub basic: bool,
    pub jwks: Jwks,
}

pub struct Provider {
    pub id: String,
    pub name: String,
    /// The configured issuer without a trailing `/`.
    pub issuer: String,
    pub client_id: String,
    pub(crate) client_secret: Secret,
    pub(crate) agent: ureq::Agent,
    /// Tests only: discovery from this base, and http allowed on its host.
    test_base: Mutex<Option<Url>>,
    pub(crate) ready: RwLock<Option<Ready>>,
    /// When an unknown `kid` last refetched the JWKS; `None` until it first does.
    jwks_refreshed: Mutex<Option<i64>>,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    #[serde(default)]
    token_endpoint_auth_methods_supported: Option<Vec<String>>,
}

fn get_bytes(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    let mut res = agent.get(url).call().map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("HTTP {} from {url}", res.status().as_u16()));
    }
    res.body_mut()
        .with_config()
        .limit(BODY_LIMIT)
        .read_to_vec()
        .map_err(|e| e.to_string())
}

impl Provider {
    fn new(c: &OidcProvider) -> Provider {
        Provider {
            id: c.id.clone(),
            name: c.name.clone(),
            issuer: c.issuer.as_str().trim_end_matches('/').to_string(),
            client_id: c.client_id.clone(),
            client_secret: c.client_secret.clone(),
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(TIMEOUT))
                .http_status_as_error(false)
                .build()
                .into(),
            test_base: Mutex::new(None),
            ready: RwLock::new(None),
            jwks_refreshed: Mutex::new(None),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.ready
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
    }

    pub(crate) fn ready_state(&self) -> Option<Ready> {
        self.ready.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// An endpoint from discovery: https, or http on the test server's host.
    fn endpoint(&self, raw: &str) -> Result<Url, String> {
        let url = Url::parse(raw).map_err(|e| format!("{raw}: {e}"))?;
        let test = self
            .test_base
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let test_ok =
            test.is_some_and(|t| t.host_str() == url.host_str() && t.port() == url.port());
        if url.scheme() == "https" || (url.scheme() == "http" && test_ok) {
            Ok(url)
        } else {
            Err(format!("{raw} is not https"))
        }
    }

    fn discover(&self) -> Result<Ready, String> {
        let base = match self
            .test_base
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            Some(t) => t.as_str().trim_end_matches('/').to_string(),
            None => self.issuer.clone(),
        };
        let doc = get_bytes(
            &self.agent,
            &format!("{base}/.well-known/openid-configuration"),
        )?;
        let d: Discovery = serde_json::from_slice(&doc).map_err(|e| format!("discovery: {e}"))?;
        if d.issuer.trim_end_matches('/') != self.issuer {
            return Err(format!(
                "discovery names issuer {}, not {}",
                d.issuer, self.issuer
            ));
        }
        let jwks_uri = self.endpoint(&d.jwks_uri)?;
        let jwks = Jwks::parse(&get_bytes(&self.agent, jwks_uri.as_str())?)?;
        let methods = d.token_endpoint_auth_methods_supported.unwrap_or_default();
        let has = |m: &str| methods.iter().any(|x| x == m);
        Ok(Ready {
            authorization: self.endpoint(&d.authorization_endpoint)?,
            token: self.endpoint(&d.token_endpoint)?,
            jwks_uri,
            basic: has("client_secret_basic") && !has("client_secret_post"),
            jwks,
        })
    }

    /// One discovery and JWKS fetch; logs the outcome.
    async fn load(self: &Arc<Self>) {
        let me = self.clone();
        let got = tokio::task::spawn_blocking(move || me.discover())
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        match got {
            Ok(ready) => {
                *self.ready.write().unwrap_or_else(|p| p.into_inner()) = Some(ready);
                tracing::info!(event = "oidc.ready", provider = %self.id);
            }
            Err(error) => {
                tracing::warn!(event = "oidc.unreachable", provider = %self.id, error = %error);
            }
        }
    }

    /// Verifies an ID token. An unknown `kid` refetches the JWKS once, at most once a minute, to follow rotation.
    pub async fn verify_id_token(
        self: &Arc<Self>,
        token: &str,
        nonce: &str,
        now: i64,
    ) -> Result<Identity, TokenError> {
        let Some(ready) = self.ready_state() else {
            return Err(TokenError::UnknownKey);
        };
        let expect = Expect {
            issuer: &self.issuer,
            client_id: &self.client_id,
            nonce,
            now,
        };
        match jwt::verify(token, &ready.jwks, &expect) {
            Err(TokenError::UnknownKey) => {}
            other => return other,
        }
        {
            let mut last = self
                .jwks_refreshed
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if last.is_some_and(|t| now - t < JWKS_REFRESH_SECS) {
                return Err(TokenError::UnknownKey);
            }
            *last = Some(now);
        }
        let (agent, uri) = (self.agent.clone(), ready.jwks_uri.clone());
        let fresh =
            tokio::task::spawn_blocking(move || Jwks::parse(&get_bytes(&agent, uri.as_str())?))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
        let jwks = match fresh {
            Ok(jwks) => jwks,
            Err(error) => {
                tracing::warn!(event = "oidc.unreachable", provider = %self.id, error = %error);
                return Err(TokenError::UnknownKey);
            }
        };
        if let Some(r) = self
            .ready
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
        {
            r.jwks = jwks.clone();
        }
        jwt::verify(token, &jwks, &expect)
    }
}

/// Every configured OIDC provider, and the key that signs pending-flow cookies.
pub struct Oidc {
    providers: Vec<Arc<Provider>>,
    pub(crate) mac_key: [u8; 32],
}

impl Oidc {
    pub fn new(list: &[OidcProvider]) -> Oidc {
        let mut mac_key = [0u8; 32];
        getrandom::fill(&mut mac_key).expect("system random source");
        Oidc {
            providers: list.iter().map(|c| Arc::new(Provider::new(c))).collect(),
            mac_key,
        }
    }

    pub fn get(&self, id: &str) -> Option<&Arc<Provider>> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// Id and button name of each provider that is ready, in configuration order.
    pub fn ready(&self) -> Vec<(&str, &str)> {
        self.providers
            .iter()
            .filter(|p| p.is_ready())
            .map(|p| (p.id.as_str(), p.name.as_str()))
            .collect()
    }

    /// Loads each provider that is not ready yet.
    pub async fn refresh_missing(&self) {
        for p in self.providers.iter().filter(|p| !p.is_ready()) {
            p.load().await;
        }
    }

    /// Loads now, then every minute while any provider is still missing. Never blocks startup.
    pub fn spawn_retry(self: &Arc<Self>) {
        if self.providers.is_empty() {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                me.refresh_missing().await;
                if me.providers.iter().all(|p| p.is_ready()) {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(RETRY_SECS)).await;
            }
        });
    }

    /// Tests only: discovery for `id` comes from `base`, and http is allowed on that host. Nothing in the
    /// environment reaches this.
    #[doc(hidden)]
    pub fn use_test_server(&self, id: &str, base: &str) {
        if let Some(p) = self.get(id) {
            *p.test_base.lock().unwrap_or_else(|e| e.into_inner()) = Url::parse(base).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use serde_json::{Value, json};

    use super::*;
    use crate::auth::oidc::jwt::testing::{Key, RSA_A, RSA_B, rsa};
    use crate::config::{Config, Vars};

    #[derive(Default)]
    struct Fake {
        issuer: String,
        jwks: Value,
        down: bool,
        http_jwks: bool,
        discovery_hits: usize,
        jwks_hits: usize,
        paths: Vec<String>,
    }

    async fn fake(issuer: &str, jwks: Value) -> (String, Arc<Mutex<Fake>>) {
        let state = Arc::new(Mutex::new(Fake {
            issuer: issuer.into(),
            jwks,
            ..Default::default()
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (a, b, base2) = (state.clone(), state.clone(), base.clone());
        let app = axum::Router::new()
            .route(
                "/{*rest}",
                get(move |uri: axum::http::Uri| {
                    let (a, base) = (a.clone(), base2.clone());
                    async move {
                        let mut f = a.lock().unwrap();
                        f.paths.push(uri.path().to_string());
                        if f.down {
                            return StatusCode::SERVICE_UNAVAILABLE.into_response();
                        }
                        if !uri.path().ends_with("/.well-known/openid-configuration") {
                            return StatusCode::NOT_FOUND.into_response();
                        }
                        f.discovery_hits += 1;
                        let jwks_uri = if f.http_jwks {
                            "http://idp.example.com/jwks".to_string()
                        } else {
                            format!("{base}/jwks")
                        };
                        axum::Json(json!({
                            "issuer": f.issuer,
                            "authorization_endpoint": format!("{base}/authorize"),
                            "token_endpoint": format!("{base}/token"),
                            "jwks_uri": jwks_uri,
                        }))
                        .into_response()
                    }
                }),
            )
            .route(
                "/jwks",
                get(move || {
                    let b = b.clone();
                    async move {
                        let mut f = b.lock().unwrap();
                        f.jwks_hits += 1;
                        if f.down {
                            return StatusCode::SERVICE_UNAVAILABLE.into_response();
                        }
                        axum::Json(f.jwks.clone()).into_response()
                    }
                }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, state)
    }

    fn oidc(issuer: &str) -> Oidc {
        let vars: Vars = [
            ("CAIRN_AUTH_METHODS", "oidc"),
            ("CAIRN_PUBLIC_URL", "https://files.example.com"),
            ("CAIRN_OIDC_PROVIDERS", "fake"),
            ("CAIRN_OIDC_FAKE_ISSUER", issuer),
            ("CAIRN_OIDC_FAKE_CLIENT_ID", "cairn-client"),
            ("CAIRN_OIDC_FAKE_CLIENT_SECRET", "client-secret-for-tests"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        Oidc::new(&Config::from_vars(&vars).unwrap().auth.oidc)
    }

    fn keys(list: &[(&Key, &str)]) -> Value {
        json!({ "keys": list.iter().map(|(k, kid)| k.jwk(kid)).collect::<Vec<_>>() })
    }

    async fn loaded(issuer_cfg: &str, issuer_doc: &str, jwks: Value) -> (Oidc, Arc<Mutex<Fake>>) {
        let (base, f) = fake(issuer_doc, jwks).await;
        let o = oidc(issuer_cfg);
        o.use_test_server("fake", &base);
        o.refresh_missing().await;
        (o, f)
    }

    #[tokio::test]
    async fn discovery_loads() {
        let k = rsa(RSA_A);
        let (o, _) = loaded(
            "https://idp.example.com",
            "https://idp.example.com",
            keys(&[(&k, "a")]),
        )
        .await;
        assert!(o.get("fake").unwrap().is_ready());
        assert_eq!(o.ready(), vec![("fake", "fake")]);
    }

    #[tokio::test]
    async fn discovery_url_from_issuer() {
        for cfg in ["https://idp.example.com/", "https://idp.example.com"] {
            let (_, f) = loaded(cfg, "https://idp.example.com", json!({ "keys": [] })).await;
            let f = f.lock().unwrap();
            assert_eq!(f.discovery_hits, 1, "{cfg}");
            assert_eq!(f.paths[0], "/.well-known/openid-configuration", "{cfg}");
        }
    }

    #[tokio::test]
    async fn issuer_slash_either_way() {
        for (cfg, doc) in [
            ("https://idp.example.com", "https://idp.example.com/"),
            ("https://idp.example.com/", "https://idp.example.com"),
        ] {
            let (o, _) = loaded(cfg, doc, json!({ "keys": [] })).await;
            assert!(o.get("fake").unwrap().is_ready(), "{cfg} / {doc}");
        }
    }

    #[tokio::test]
    async fn issuer_mismatch_not_ready() {
        let (o, _) = loaded(
            "https://idp.example.com",
            "https://evil.example.com",
            json!({ "keys": [] }),
        )
        .await;
        assert!(!o.get("fake").unwrap().is_ready());
        assert!(o.ready().is_empty());
    }

    #[tokio::test]
    async fn http_endpoint_refused() {
        let (base, f) = fake("https://idp.example.com", json!({ "keys": [] })).await;
        f.lock().unwrap().http_jwks = true;
        let o = oidc("https://idp.example.com");
        // The test server may use http; a jwks_uri on any other host may not.
        o.use_test_server("fake", &base);
        o.refresh_missing().await;
        assert!(!o.get("fake").unwrap().is_ready());
    }

    #[tokio::test]
    async fn unreachable_then_retry() {
        let (base, f) = fake("https://idp.example.com", json!({ "keys": [] })).await;
        f.lock().unwrap().down = true;
        let o = oidc("https://idp.example.com");
        o.use_test_server("fake", &base);
        o.refresh_missing().await;
        assert!(!o.get("fake").unwrap().is_ready());
        f.lock().unwrap().down = false;
        o.refresh_missing().await;
        assert!(o.get("fake").unwrap().is_ready());
    }

    fn claims(now: i64) -> Value {
        json!({
            "iss": "https://idp.example.com", "aud": "cairn-client",
            "exp": now + 300, "iat": now, "nonce": "n",
            "email": "ana@example.com", "email_verified": true,
        })
    }

    #[tokio::test]
    async fn unknown_kid_refetches_once_a_minute() {
        let (a, b) = (rsa(RSA_A), rsa(RSA_B));
        let (o, f) = loaded(
            "https://idp.example.com",
            "https://idp.example.com",
            keys(&[(&a, "a")]),
        )
        .await;
        let p = o.get("fake").unwrap().clone();
        let now = crate::now();
        assert!(
            p.verify_id_token(&a.sign(Some("a"), &claims(now)), "n", now)
                .await
                .is_ok()
        );
        f.lock().unwrap().jwks = keys(&[(&b, "b")]);
        let hits = f.lock().unwrap().jwks_hits;
        assert!(
            p.verify_id_token(&b.sign(Some("b"), &claims(now)), "n", now)
                .await
                .is_ok()
        );
        assert_eq!(f.lock().unwrap().jwks_hits, hits + 1);
        let t = a.sign(Some("c"), &claims(now));
        assert_eq!(
            p.verify_id_token(&t, "n", now + 30).await,
            Err(TokenError::UnknownKey)
        );
        assert_eq!(f.lock().unwrap().jwks_hits, hits + 1);
    }
}
