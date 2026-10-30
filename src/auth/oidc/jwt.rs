//! ID token verification: RS256 or ES256 signatures against the provider's JWKS, then the claims Cairn relies on.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::signature::{
    ECDSA_P256_SHA256_FIXED, RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents, UnparsedPublicKey,
};
use serde::Deserialize;
use serde_json::Value;

/// How far an expired token is still accepted, for clocks a little apart.
const LEEWAY_SECS: i64 = 60;
/// How far in the future `iat` may be.
const IAT_AHEAD_SECS: i64 = 300;

#[derive(Debug, Clone)]
enum Material {
    Rsa {
        n: Vec<u8>,
        e: Vec<u8>,
    },
    /// `0x04 || x || y`, P-256.
    Ec(Vec<u8>),
}

#[derive(Debug, Clone)]
struct Jwk {
    kid: Option<String>,
    material: Material,
}

/// The signing keys of a provider; other key types and encryption keys are left out.
#[derive(Debug, Clone, Default)]
pub struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct RawJwks {
    keys: Vec<Value>,
}

fn b64(v: &Value) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(v.as_str()?).ok()
}

impl Jwks {
    pub fn parse(json: &[u8]) -> Result<Jwks, String> {
        let raw: RawJwks = serde_json::from_slice(json).map_err(|e| format!("JWKS: {e}"))?;
        let keys = raw
            .keys
            .iter()
            .filter(|k| k.get("use").is_none_or(|u| u == "sig"))
            .filter_map(|k| {
                let material = match k["kty"].as_str()? {
                    "RSA" => Material::Rsa {
                        n: b64(&k["n"])?,
                        e: b64(&k["e"])?,
                    },
                    "EC" if k["crv"] == "P-256" => {
                        let (x, y) = (b64(&k["x"])?, b64(&k["y"])?);
                        if x.len() != 32 || y.len() != 32 {
                            return None;
                        }
                        Material::Ec([&[4u8][..], &x, &y].concat())
                    }
                    _ => return None,
                };
                Some(Jwk {
                    kid: k["kid"].as_str().map(String::from),
                    material,
                })
            })
            .collect();
        Ok(Jwks { keys })
    }
}

pub struct Expect<'a> {
    pub issuer: &'a str,
    pub client_id: &'a str,
    pub nonce: &'a str,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub email: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenError {
    Malformed,
    Alg,
    UnknownKey,
    Signature,
    Issuer,
    Audience,
    Azp,
    Expired,
    Iat,
    Nonce,
    EmailMissing,
    EmailUnverified,
}

impl TokenError {
    pub fn reason(&self) -> &'static str {
        match self {
            TokenError::Malformed | TokenError::Signature => "signature",
            TokenError::Alg => "alg",
            TokenError::UnknownKey => "unknown_key",
            TokenError::Issuer => "issuer",
            TokenError::Audience => "audience",
            TokenError::Azp => "azp",
            TokenError::Expired => "expired",
            TokenError::Iat => "iat",
            TokenError::Nonce => "nonce",
            TokenError::EmailMissing => "email_missing",
            TokenError::EmailUnverified => "email_unverified",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Alg {
    Rs256,
    Es256,
}

fn part(s: &str) -> Result<Value, TokenError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| TokenError::Malformed)?;
    serde_json::from_slice(&bytes).map_err(|_| TokenError::Malformed)
}

fn signed_by(alg: Alg, key: &Jwk, input: &[u8], sig: &[u8]) -> bool {
    match (alg, &key.material) {
        (Alg::Rs256, Material::Rsa { n, e }) => RsaPublicKeyComponents { n, e }
            .verify(&RSA_PKCS1_2048_8192_SHA256, input, sig)
            .is_ok(),
        (Alg::Es256, Material::Ec(point)) => {
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point)
                .verify(input, sig)
                .is_ok()
        }
        _ => false,
    }
}

fn fits(alg: Alg, key: &Jwk) -> bool {
    matches!(
        (alg, &key.material),
        (Alg::Rs256, Material::Rsa { .. }) | (Alg::Es256, Material::Ec(_))
    )
}

fn trim(issuer: &str) -> &str {
    issuer.trim_end_matches('/')
}

pub fn verify(token: &str, jwks: &Jwks, expect: &Expect) -> Result<Identity, TokenError> {
    let mut parts = token.split('.');
    let (Some(h), Some(p), Some(sig), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(TokenError::Malformed);
    };
    let header = part(h)?;
    let alg = match header["alg"].as_str() {
        Some("RS256") => Alg::Rs256,
        Some("ES256") => Alg::Es256,
        _ => return Err(TokenError::Alg),
    };
    let kid = header["kid"].as_str();
    let candidates: Vec<&Jwk> = jwks
        .keys
        .iter()
        .filter(|k| fits(alg, k) && (kid.is_none() || k.kid.as_deref() == kid))
        .collect();
    if candidates.is_empty() {
        return Err(TokenError::UnknownKey);
    }
    let sig = URL_SAFE_NO_PAD
        .decode(sig)
        .map_err(|_| TokenError::Malformed)?;
    let input = &token[..h.len() + 1 + p.len()];
    if !candidates
        .iter()
        .any(|k| signed_by(alg, k, input.as_bytes(), &sig))
    {
        return Err(TokenError::Signature);
    }
    let c = part(p)?;
    if c["iss"].as_str().map(trim) != Some(trim(expect.issuer)) {
        return Err(TokenError::Issuer);
    }
    let aud_ok = match &c["aud"] {
        Value::String(a) => a == expect.client_id,
        Value::Array(list) => list.iter().any(|a| a == expect.client_id),
        _ => false,
    };
    if !aud_ok {
        return Err(TokenError::Audience);
    }
    if let Some(azp) = c.get("azp")
        && azp != expect.client_id
    {
        return Err(TokenError::Azp);
    }
    match c["exp"].as_i64() {
        Some(exp) if exp > expect.now - LEEWAY_SECS => {}
        _ => return Err(TokenError::Expired),
    }
    if c["iat"]
        .as_i64()
        .is_none_or(|iat| iat > expect.now + IAT_AHEAD_SECS)
    {
        return Err(TokenError::Iat);
    }
    if c["nonce"]
        .as_str()
        .is_none_or(|n| !crate::config::ct_eq(n.as_bytes(), expect.nonce.as_bytes()))
    {
        return Err(TokenError::Nonce);
    }
    let Some(email) = c["email"].as_str() else {
        return Err(TokenError::EmailMissing);
    };
    // A boolean true only: some providers let anyone put any address on an account.
    if c["email_verified"] != Value::Bool(true) {
        return Err(TokenError::EmailUnverified);
    }
    Ok(Identity {
        email: email.to_string(),
        name: c["name"].as_str().map(String::from),
    })
}

#[cfg(test)]
pub(crate) mod testing {
    //! Keys and token signing for tests. The RSA keys are made for the tests and hold nothing else.

    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use ring::rand::SystemRandom;
    use ring::signature::{self, EcdsaKeyPair, KeyPair, RsaKeyPair};
    use serde_json::{Value, json};

    pub const RSA_A: &[u8] = include_bytes!("../../../tests/fixtures/test-rsa-2048-a.pk8");
    pub const RSA_B: &[u8] = include_bytes!("../../../tests/fixtures/test-rsa-2048-b.pk8");

    pub enum Key {
        Rsa(RsaKeyPair),
        Ec(EcdsaKeyPair),
    }

    pub fn rsa(der: &[u8]) -> Key {
        Key::Rsa(RsaKeyPair::from_pkcs8(der).unwrap())
    }

    pub fn ec() -> Key {
        let rng = SystemRandom::new();
        let alg = &signature::ECDSA_P256_SHA256_FIXED_SIGNING;
        let pk8 = EcdsaKeyPair::generate_pkcs8(alg, &rng).unwrap();
        Key::Ec(EcdsaKeyPair::from_pkcs8(alg, pk8.as_ref(), &rng).unwrap())
    }

    fn b64(b: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(b)
    }

    impl Key {
        pub fn jwk(&self, kid: &str) -> Value {
            match self {
                Key::Rsa(k) => {
                    let c = signature::RsaPublicKeyComponents::<Vec<u8>>::from(k.public());
                    json!({ "kty": "RSA", "kid": kid, "use": "sig", "alg": "RS256", "n": b64(&c.n), "e": b64(&c.e) })
                }
                Key::Ec(k) => {
                    let p = k.public_key().as_ref();
                    json!({ "kty": "EC", "kid": kid, "crv": "P-256", "x": b64(&p[1..33]), "y": b64(&p[33..65]) })
                }
            }
        }

        pub fn alg(&self) -> &'static str {
            match self {
                Key::Rsa(_) => "RS256",
                Key::Ec(_) => "ES256",
            }
        }

        /// A signed token; `kid` None leaves it out of the header.
        pub fn sign(&self, kid: Option<&str>, claims: &Value) -> String {
            let mut header = json!({ "alg": self.alg(), "typ": "JWT" });
            if let Some(kid) = kid {
                header["kid"] = json!(kid);
            }
            let input = format!(
                "{}.{}",
                b64(header.to_string().as_bytes()),
                b64(claims.to_string().as_bytes())
            );
            let rng = SystemRandom::new();
            let sig = match self {
                Key::Rsa(k) => {
                    let mut sig = vec![0; k.public().modulus_len()];
                    k.sign(
                        &signature::RSA_PKCS1_SHA256,
                        &rng,
                        input.as_bytes(),
                        &mut sig,
                    )
                    .unwrap();
                    sig
                }
                Key::Ec(k) => k.sign(&rng, input.as_bytes()).unwrap().as_ref().to_vec(),
            };
            format!("{input}.{}", b64(&sig))
        }
    }

    /// A token with any header, signed with HMAC-SHA256 under `secret`, or unsigned when `secret` is None.
    pub fn forged(header: &Value, claims: &Value, secret: Option<&[u8]>) -> String {
        let input = format!(
            "{}.{}",
            b64(header.to_string().as_bytes()),
            b64(claims.to_string().as_bytes())
        );
        let sig = match secret {
            Some(s) => {
                let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, s);
                b64(ring::hmac::sign(&key, input.as_bytes()).as_ref())
            }
            None => String::new(),
        };
        format!("{input}.{sig}")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::testing::*;
    use super::*;

    const NOW: i64 = 1_800_000_000;
    const ISS: &str = "https://idp.example.com";

    fn claims() -> Value {
        json!({
            "iss": ISS, "aud": "cairn-client", "sub": "1",
            "exp": NOW + 300, "iat": NOW - 10, "nonce": "n-123",
            "email": "Ana@Example.COM", "email_verified": true, "name": "Ana",
        })
    }

    fn with(changes: Value) -> Value {
        let mut c = claims();
        for (k, v) in changes.as_object().unwrap() {
            if v.is_null() {
                c.as_object_mut().unwrap().remove(k);
            } else {
                c[k] = v.clone();
            }
        }
        c
    }

    fn jwks(keys: &[(&Key, &str)]) -> Jwks {
        let keys: Vec<Value> = keys.iter().map(|(k, kid)| k.jwk(kid)).collect();
        Jwks::parse(json!({ "keys": keys }).to_string().as_bytes()).unwrap()
    }

    fn expect() -> Expect<'static> {
        Expect {
            issuer: ISS,
            client_id: "cairn-client",
            nonce: "n-123",
            now: NOW,
        }
    }

    fn check(key: &Key, c: Value) -> Result<Identity, TokenError> {
        verify(&key.sign(Some("k1"), &c), &jwks(&[(key, "k1")]), &expect())
    }

    #[test]
    fn rs256_ok() {
        let id = check(&rsa(RSA_A), claims()).unwrap();
        assert_eq!(id.email, "Ana@Example.COM");
        assert_eq!(id.name.as_deref(), Some("Ana"));
    }

    #[test]
    fn es256_ok() {
        assert!(check(&ec(), claims()).is_ok());
    }

    #[test]
    fn alg_none_refused() {
        let k = rsa(RSA_A);
        let t = forged(&json!({ "alg": "none", "kid": "k1" }), &claims(), None);
        assert_eq!(
            verify(&t, &jwks(&[(&k, "k1")]), &expect()),
            Err(TokenError::Alg)
        );
    }

    #[test]
    fn hs256_with_client_secret_refused() {
        let k = rsa(RSA_A);
        let t = forged(
            &json!({ "alg": "HS256", "kid": "k1" }),
            &claims(),
            Some(b"client-secret-for-tests"),
        );
        assert_eq!(
            verify(&t, &jwks(&[(&k, "k1")]), &expect()),
            Err(TokenError::Alg)
        );
    }

    #[test]
    fn wrong_signature() {
        let k = rsa(RSA_A);
        let t = k.sign(Some("k1"), &claims());
        let mut parts: Vec<&str> = t.split('.').collect();
        let other = k.sign(Some("k1"), &with(json!({ "email": "eve@example.com" })));
        let payload = other.split('.').nth(1).unwrap().to_string();
        parts[1] = &payload;
        let t = parts.join(".");
        assert_eq!(
            verify(&t, &jwks(&[(&k, "k1")]), &expect()),
            Err(TokenError::Signature)
        );
    }

    #[test]
    fn unknown_kid() {
        let k = rsa(RSA_A);
        let t = k.sign(Some("k9"), &claims());
        assert_eq!(
            verify(&t, &jwks(&[(&k, "k1")]), &expect()),
            Err(TokenError::UnknownKey)
        );
    }

    #[test]
    fn no_kid_tries_all() {
        let (a, b) = (rsa(RSA_A), rsa(RSA_B));
        let t = b.sign(None, &claims());
        assert!(verify(&t, &jwks(&[(&a, "a"), (&b, "b")]), &expect()).is_ok());
    }

    #[test]
    fn wrong_issuer() {
        let k = rsa(RSA_A);
        assert_eq!(
            check(&k, with(json!({ "iss": "https://evil.example.com" }))).unwrap_err(),
            TokenError::Issuer
        );
    }

    #[test]
    fn issuer_trailing_slash() {
        let k = rsa(RSA_A);
        assert!(check(&k, with(json!({ "iss": "https://idp.example.com/" }))).is_ok());
        let t = k.sign(Some("k1"), &claims());
        let e = Expect {
            issuer: "https://idp.example.com/",
            ..expect()
        };
        assert!(verify(&t, &jwks(&[(&k, "k1")]), &e).is_ok());
    }

    #[test]
    fn aud_array_ok() {
        assert!(
            check(
                &rsa(RSA_A),
                with(json!({ "aud": ["other", "cairn-client"] }))
            )
            .is_ok()
        );
    }

    #[test]
    fn wrong_aud() {
        let k = rsa(RSA_A);
        assert_eq!(
            check(&k, with(json!({ "aud": "other" }))).unwrap_err(),
            TokenError::Audience
        );
        assert_eq!(
            check(&k, with(json!({ "aud": ["other"] }))).unwrap_err(),
            TokenError::Audience
        );
    }

    #[test]
    fn azp_mismatch() {
        let k = rsa(RSA_A);
        assert_eq!(
            check(&k, with(json!({ "azp": "other" }))).unwrap_err(),
            TokenError::Azp
        );
        assert!(check(&k, with(json!({ "azp": "cairn-client" }))).is_ok());
    }

    #[test]
    fn expired() {
        let k = rsa(RSA_A);
        assert_eq!(
            check(&k, with(json!({ "exp": NOW - 61 }))).unwrap_err(),
            TokenError::Expired
        );
        assert!(check(&k, with(json!({ "exp": NOW - 59 }))).is_ok());
    }

    #[test]
    fn iat_ahead() {
        let k = rsa(RSA_A);
        assert_eq!(
            check(&k, with(json!({ "iat": NOW + 301 }))).unwrap_err(),
            TokenError::Iat
        );
        assert!(check(&k, with(json!({ "iat": NOW + 299 }))).is_ok());
    }

    #[test]
    fn wrong_nonce() {
        assert_eq!(
            check(&rsa(RSA_A), with(json!({ "nonce": "other" }))).unwrap_err(),
            TokenError::Nonce
        );
    }

    #[test]
    fn email_missing() {
        assert_eq!(
            check(&rsa(RSA_A), with(json!({ "email": null }))).unwrap_err(),
            TokenError::EmailMissing
        );
    }

    #[test]
    fn email_unverified() {
        let k = rsa(RSA_A);
        for v in [json!(false), Value::Null, json!("true")] {
            assert_eq!(
                check(&k, with(json!({ "email_verified": v.clone() }))).unwrap_err(),
                TokenError::EmailUnverified,
                "{v}"
            );
        }
    }

    #[test]
    fn jwks_skips_enc_keys() {
        let k = rsa(RSA_A);
        let mut jwk = k.jwk("k1");
        jwk["use"] = json!("enc");
        let set = Jwks::parse(json!({ "keys": [jwk] }).to_string().as_bytes()).unwrap();
        let t = k.sign(Some("k1"), &claims());
        assert_eq!(verify(&t, &set, &expect()), Err(TokenError::UnknownKey));
    }
}
