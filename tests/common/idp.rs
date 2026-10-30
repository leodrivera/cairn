//! A fake OpenID Connect provider: discovery, JWKS and a token endpoint that hands out ID tokens the test shapes.
//! Its RSA keys are made for the tests and hold nothing else.

use std::sync::{Arc, Mutex};

use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::SystemRandom;
use ring::signature::{self, EcdsaKeyPair, KeyPair, RsaKeyPair};
use serde_json::{Value, json};

const RSA_A: &[u8] = include_bytes!("../fixtures/test-rsa-2048-a.pk8");
const RSA_B: &[u8] = include_bytes!("../fixtures/test-rsa-2048-b.pk8");
pub const CLIENT_SECRET: &str = "client-secret-for-tests";

struct State {
    issuer: String,
    claims: Value,
    alg: String,
    rsa: RsaKeyPair,
    kid: String,
    ec: EcdsaKeyPair,
    auth_methods: Option<Vec<String>>,
    nonce: String,
    token_requests: Vec<(HeaderMap, String)>,
    hits: Vec<String>,
    down: bool,
}

pub struct FakeIdp {
    pub base: String,
    state: Arc<Mutex<State>>,
}

fn b64(b: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(b)
}

fn rsa_jwk(k: &RsaKeyPair, kid: &str) -> Value {
    let c = signature::RsaPublicKeyComponents::<Vec<u8>>::from(k.public());
    json!({ "kty": "RSA", "kid": kid, "use": "sig", "alg": "RS256", "n": b64(&c.n), "e": b64(&c.e) })
}

fn ec_jwk(k: &EcdsaKeyPair) -> Value {
    let p = k.public_key().as_ref();
    json!({ "kty": "EC", "kid": "ec", "crv": "P-256", "x": b64(&p[1..33]), "y": b64(&p[33..65]) })
}

impl State {
    fn id_token(&self) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let mut claims = json!({
            "iss": self.issuer, "aud": "cairn-client", "sub": "1",
            "exp": now + 300, "iat": now, "nonce": self.nonce,
            "email": "ana@example.com", "email_verified": true,
        });
        for (k, v) in self.claims.as_object().into_iter().flatten() {
            if v.is_null() {
                claims.as_object_mut().unwrap().remove(k);
            } else {
                claims[k] = v.clone();
            }
        }
        let kid = if self.alg == "ES256" {
            "ec"
        } else {
            self.kid.as_str()
        };
        let header = json!({ "alg": self.alg, "kid": kid, "typ": "JWT" });
        let input = format!(
            "{}.{}",
            b64(header.to_string().as_bytes()),
            b64(claims.to_string().as_bytes())
        );
        let rng = SystemRandom::new();
        let sig = match self.alg.as_str() {
            "RS256" => {
                let mut sig = vec![0; self.rsa.public().modulus_len()];
                self.rsa
                    .sign(
                        &signature::RSA_PKCS1_SHA256,
                        &rng,
                        input.as_bytes(),
                        &mut sig,
                    )
                    .unwrap();
                b64(&sig)
            }
            "ES256" => b64(self.ec.sign(&rng, input.as_bytes()).unwrap().as_ref()),
            "HS256" => {
                let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, CLIENT_SECRET.as_bytes());
                b64(ring::hmac::sign(&key, input.as_bytes()).as_ref())
            }
            _ => String::new(),
        };
        format!("{input}.{sig}")
    }
}

impl FakeIdp {
    pub async fn start(issuer: &str) -> FakeIdp {
        let rng = SystemRandom::new();
        let alg = &signature::ECDSA_P256_SHA256_FIXED_SIGNING;
        let pk8 = EcdsaKeyPair::generate_pkcs8(alg, &rng).unwrap();
        let state = Arc::new(Mutex::new(State {
            issuer: issuer.into(),
            claims: json!({}),
            alg: "RS256".into(),
            rsa: RsaKeyPair::from_pkcs8(RSA_A).unwrap(),
            kid: "a".into(),
            ec: EcdsaKeyPair::from_pkcs8(alg, pk8.as_ref(), &rng).unwrap(),
            auth_methods: None,
            nonce: String::new(),
            token_requests: Vec::new(),
            hits: Vec::new(),
            down: false,
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (s1, s2, s3, b) = (state.clone(), state.clone(), state.clone(), base.clone());
        let app = axum::Router::new()
            .route(
                "/.well-known/openid-configuration",
                get(move || async move {
                    let mut s = s1.lock().unwrap();
                    s.hits.push("discovery".into());
                    if s.down {
                        return StatusCode::SERVICE_UNAVAILABLE.into_response();
                    }
                    let mut doc = json!({
                        "issuer": s.issuer,
                        "authorization_endpoint": format!("{b}/authorize"),
                        "token_endpoint": format!("{b}/token"),
                        "jwks_uri": format!("{b}/jwks"),
                    });
                    if let Some(m) = &s.auth_methods {
                        doc["token_endpoint_auth_methods_supported"] = json!(m);
                    }
                    axum::Json(doc).into_response()
                }),
            )
            .route(
                "/jwks",
                get(move || async move {
                    let mut s = s2.lock().unwrap();
                    s.hits.push("jwks".into());
                    axum::Json(json!({ "keys": [rsa_jwk(&s.rsa, &s.kid), ec_jwk(&s.ec)] }))
                        .into_response()
                }),
            )
            .route(
                "/token",
                post(move |headers: HeaderMap, body: String| async move {
                    let mut s = s3.lock().unwrap();
                    s.hits.push("token".into());
                    s.token_requests.push((headers, body));
                    axum::Json(json!({ "access_token": "at", "token_type": "Bearer", "id_token": s.id_token() }))
                        .into_response()
                }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeIdp { base, state }
    }

    /// Claims merged over the defaults; a null removes one.
    pub fn set_claims(&self, claims: Value) {
        self.state.lock().unwrap().claims = claims;
    }
    pub fn set_alg(&self, alg: &str) {
        self.state.lock().unwrap().alg = alg.into();
    }
    /// Signs with a new RSA key under a new `kid`; the old one leaves the JWKS.
    pub fn rotate_key(&self) {
        let mut s = self.state.lock().unwrap();
        s.rsa = RsaKeyPair::from_pkcs8(RSA_B).unwrap();
        s.kid = "b".into();
    }
    pub fn set_auth_methods(&self, m: Option<Vec<&str>>) {
        self.state.lock().unwrap().auth_methods =
            m.map(|v| v.into_iter().map(String::from).collect());
    }
    /// The browser went to the authorization endpoint: the next ID token carries that request's nonce.
    pub fn authorize(&self, location: &str) {
        let url = url::Url::parse(location).unwrap();
        let nonce = url
            .query_pairs()
            .find(|(k, _)| k == "nonce")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default();
        self.state.lock().unwrap().nonce = nonce;
    }
    pub fn token_requests(&self) -> Vec<(HeaderMap, String)> {
        self.state.lock().unwrap().token_requests.clone()
    }
    pub fn hits(&self, what: &str) -> usize {
        self.state
            .lock()
            .unwrap()
            .hits
            .iter()
            .filter(|h| *h == what)
            .count()
    }
    pub fn set_down(&self, down: bool) {
        self.state.lock().unwrap().down = down;
    }
}
