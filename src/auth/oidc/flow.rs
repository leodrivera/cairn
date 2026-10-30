//! A sign-in in progress: `state`, `nonce` and the PKCE verifier, kept in a short-lived cookie signed with a key the
//! process makes at start, so a cookie planted by a sibling site cannot choose them.

use axum::http::HeaderValue;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::hmac;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const COOKIE: &str = "cairn_oidc";
/// A sign-in must come back within this time.
pub const MAX_AGE: i64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pending {
    #[serde(rename = "p")]
    pub provider: String,
    #[serde(rename = "s")]
    pub state: String,
    #[serde(rename = "n")]
    pub nonce: String,
    #[serde(rename = "v")]
    pub verifier: String,
    #[serde(rename = "t")]
    pub created: i64,
}

fn random() -> String {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).expect("system random source");
    URL_SAFE_NO_PAD.encode(raw)
}

fn mac(key: &[u8; 32], payload: &str) -> hmac::Tag {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), payload.as_bytes())
}

impl Pending {
    pub fn new(provider: &str, now: i64) -> Pending {
        Pending {
            provider: provider.to_string(),
            state: random(),
            nonce: random(),
            verifier: random(),
            created: now,
        }
    }

    /// PKCE S256: base64url of the SHA-256 of the verifier.
    pub fn challenge(&self) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()))
    }

    pub fn seal(&self, key: &[u8; 32]) -> String {
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("serializable"));
        let tag = URL_SAFE_NO_PAD.encode(mac(key, &payload).as_ref());
        format!("{payload}.{tag}")
    }
}

/// The pending sign-in a cookie carries, if its signature holds and it is not older than ten minutes.
pub fn open(cookie: &str, key: &[u8; 32], now: i64) -> Option<Pending> {
    let (payload, tag) = cookie.split_once('.')?;
    let tag = URL_SAFE_NO_PAD.decode(tag).ok()?;
    hmac::verify(
        &hmac::Key::new(hmac::HMAC_SHA256, key),
        payload.as_bytes(),
        &tag,
    )
    .ok()?;
    let p: Pending = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    (now - p.created <= MAX_AGE && p.created <= now).then_some(p)
}

pub fn pending_cookie(value: &str, https: bool) -> HeaderValue {
    let secure = if https { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{COOKIE}={value}; Path=/auth/oidc/; HttpOnly; SameSite=Lax; Max-Age={MAX_AGE}{secure}"
    ))
    .expect("ASCII")
}

pub fn clear_pending(https: bool) -> HeaderValue {
    let secure = if https { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{COOKIE}=; Path=/auth/oidc/; HttpOnly; SameSite=Lax; Max-Age=0{secure}"
    ))
    .expect("ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];

    #[test]
    fn seal_round_trip() {
        let p = Pending::new("fake", 1000);
        assert_eq!(p.state.len(), 43);
        assert_eq!(p.nonce.len(), 43);
        assert_eq!(p.verifier.len(), 43);
        let got = open(&p.seal(&KEY), &KEY, 1000).unwrap();
        assert_eq!(
            (
                got.provider,
                got.state,
                got.nonce,
                got.verifier,
                got.created
            ),
            (p.provider, p.state, p.nonce, p.verifier, 1000)
        );
    }

    #[test]
    fn tampered_refused() {
        let sealed = Pending::new("fake", 1000).seal(&KEY);
        let (payload, mac) = sealed.split_once('.').unwrap();
        let flip = |s: &str| {
            let mut b = s.as_bytes().to_vec();
            b[3] = if b[3] == b'A' { b'B' } else { b'A' };
            String::from_utf8(b).unwrap()
        };
        assert!(open(&format!("{}.{mac}", flip(payload)), &KEY, 1000).is_none());
        assert!(open(&format!("{payload}.{}", flip(mac)), &KEY, 1000).is_none());
        assert!(open(&sealed, &[8; 32], 1000).is_none());
        assert!(open("garbage", &KEY, 1000).is_none());
    }

    #[test]
    fn expired_refused() {
        let sealed = Pending::new("fake", 1000).seal(&KEY);
        assert!(open(&sealed, &KEY, 1600).is_some());
        assert!(open(&sealed, &KEY, 1601).is_none());
    }

    #[test]
    fn challenge_is_s256() {
        let p = Pending {
            verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(),
            ..Pending::new("fake", 0)
        };
        assert_eq!(p.challenge(), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }
}
