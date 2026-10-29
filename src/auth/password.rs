//! The password rule and Argon2id hashing.

use std::sync::OnceLock;

use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Algorithm, Argon2, Params, Version};
use tokio::sync::Semaphore;

pub const POLICY: &str = "The password needs at least 8 characters, with an uppercase letter, a lowercase letter and a special character.";

/// The only accepted stored shape: fixed parameters, a 16-byte salt and a 32-byte hash (unpadded base64).
const PREFIX: &str = "$argon2id$v=19$m=7168,t=5,p=1$";

pub fn policy_ok(password: &str) -> bool {
    let n = password.chars().count();
    (8..=256).contains(&n)
        && password.chars().any(|c| c.is_lowercase())
        && password.chars().any(|c| c.is_uppercase())
        && password.chars().any(|c| !c.is_alphanumeric())
}

fn argon2() -> Argon2<'static> {
    let params = Params::new(7168, 5, 1, Some(32)).expect("fixed Argon2 parameters");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

pub fn hash_blocking(password: &str) -> String {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).expect("system random source");
    argon2()
        .hash_password_with_salt(password.as_bytes(), &salt)
        .expect("Argon2 with fixed parameters")
        .to_string()
}

/// True only for the right password and a hash in the fixed shape. A missing hash, or one with other parameters
/// (which could be chosen to stall the server), is never run: one verification against a dummy happens instead,
/// so a wrong e-mail takes as long as a wrong password.
pub fn verify_blocking(password: &str, stored: Option<&str>) -> bool {
    match stored.filter(|s| well_formed(s)) {
        Some(stored) => argon2()
            .verify_password(password.as_bytes(), stored)
            .is_ok(),
        None => {
            static DUMMY: OnceLock<String> = OnceLock::new();
            let dummy = DUMMY.get_or_init(|| hash_blocking("dummy password for timing"));
            let _ = argon2().verify_password(password.as_bytes(), dummy.as_str());
            false
        }
    }
}

fn well_formed(stored: &str) -> bool {
    let b64 = |s: &str| {
        s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    };
    match stored
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.split_once('$'))
    {
        Some((salt, hash)) => salt.len() == 22 && hash.len() == 43 && b64(salt) && b64(hash),
        None => false,
    }
}

/// Runs at most two hashes at once, off the async threads: each one holds 7 MiB for tens of milliseconds.
pub struct Hasher {
    permits: Semaphore,
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher {
    pub fn new() -> Hasher {
        Hasher {
            permits: Semaphore::new(2),
        }
    }

    pub async fn hash(&self, password: String) -> String {
        let _permit = self
            .permits
            .acquire()
            .await
            .expect("semaphore never closed");
        tokio::task::spawn_blocking(move || hash_blocking(&password))
            .await
            .expect("hash task")
    }

    pub async fn verify(&self, password: String, stored: Option<String>) -> bool {
        let _permit = self
            .permits
            .acquire()
            .await
            .expect("semaphore never closed");
        tokio::task::spawn_blocking(move || verify_blocking(&password, stored.as_deref()))
            .await
            .expect("verify task")
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn policy() {
        assert!(policy_ok("Abcdef1!"));
        assert!(policy_ok("aB!aB!aB"));
        assert!(!policy_ok("Abcdef1"), "7 characters");
        assert!(!policy_ok("abcdefg!"), "no uppercase");
        assert!(!policy_ok("ABCDEFG!"), "no lowercase");
        assert!(!policy_ok("Abcdefgh"), "no special character");
        assert!(
            !policy_ok(&format!("Ab!{}", "x".repeat(254))),
            "257 characters"
        );
        assert!(
            policy_ok(&format!("Ab!{}", "x".repeat(253))),
            "256 characters"
        );
    }

    #[test]
    fn hash_has_fixed_params() {
        assert!(hash_blocking("Abcdef1!").starts_with("$argon2id$v=19$m=7168,t=5,p=1$"));
    }

    #[test]
    fn verify_round_trip() {
        let stored = hash_blocking("Abcdef1!");
        assert!(verify_blocking("Abcdef1!", Some(&stored)));
        assert!(!verify_blocking("Abcdef1?", Some(&stored)));
    }

    #[test]
    fn verify_none_is_false() {
        assert!(!verify_blocking("Abcdef1!", None));
    }

    #[test]
    fn crafted_hash_refused_fast() {
        let start = Instant::now();
        assert!(!verify_blocking(
            "x",
            Some("$argon2id$v=19$m=4194304,t=1,p=1$c2FsdHNhbHQ$aGFzaGhhc2g")
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn two_at_a_time() {
        let hasher = std::sync::Arc::new(Hasher::new());
        let stored = hasher.hash("Abcdef1!".into()).await;
        let mut tasks = Vec::new();
        for i in 0..4 {
            let (h, s) = (hasher.clone(), stored.clone());
            let guess = if i % 2 == 0 { "Abcdef1!" } else { "wrong" };
            tasks.push(tokio::spawn(async move {
                h.verify(guess.into(), Some(s)).await
            }));
        }
        let mut results = Vec::new();
        for t in tasks {
            results.push(t.await.unwrap());
        }
        assert_eq!(results, [true, false, true, false]);
    }
}
