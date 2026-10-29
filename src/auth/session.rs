//! Sessions: a random token in the cookie, its SHA-256 in the database.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::db::Db;
use crate::users::{self, User};

/// "Keep me signed in": 30 days, sliding.
pub const KEEP_SECS: i64 = 30 * 24 * 3600;
/// Without it: 12 hours from sign-in, no slide.
pub const SHORT_SECS: i64 = 12 * 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Password,
    Oauth,
    Oidc,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Password => "password",
            Method::Oauth => "oauth",
            Method::Oidc => "oidc",
        }
    }

    fn parse(s: &str) -> Method {
        match s {
            "oauth" => Method::Oauth,
            "oidc" => Method::Oidc,
            _ => Method::Password,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub user: User,
    pub method: Method,
    pub keep: bool,
    pub expires_at: i64,
    /// The expiry just moved forward, so the cookie should be sent again.
    pub renewed: bool,
}

fn digest(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Starts a session and returns its token. Also clears out expired rows.
pub fn create(
    db: &Db,
    user_id: i64,
    method: Method,
    keep: bool,
    label: &str,
    now: i64,
) -> rusqlite::Result<String> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).expect("system random source");
    let token = URL_SAFE_NO_PAD.encode(raw);
    let expires = now + if keep { KEEP_SECS } else { SHORT_SECS };
    db.with(|c| {
        c.execute("DELETE FROM sessions WHERE expires_at <= ?1", [now])?;
        c.execute(
            "INSERT INTO sessions (id, user_id, method, keep, label, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![digest(&token), user_id, method.as_str(), keep, label, now, expires],
        )
    })?;
    Ok(token)
}

pub fn lookup(db: &Db, token: &str, now: i64) -> rusqlite::Result<Option<Session>> {
    let id = digest(token);
    let row: Option<(i64, String, bool, i64)> = db.with(|c| {
        c.query_row(
            "SELECT user_id, method, keep, expires_at FROM sessions WHERE id = ?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
    })?;
    let Some((user_id, method, keep, mut expires_at)) = row else {
        return Ok(None);
    };
    if expires_at <= now {
        db.with(|c| c.execute("DELETE FROM sessions WHERE id = ?1", [&id]))?;
        return Ok(None);
    }
    let renewed = keep && expires_at - now < KEEP_SECS / 2;
    if renewed {
        expires_at = now + KEEP_SECS;
        db.with(|c| {
            c.execute(
                "UPDATE sessions SET expires_at = ?2 WHERE id = ?1",
                params![id, expires_at],
            )
        })?;
    }
    let user = match users::by_id(db, user_id) {
        Ok(Some(u)) => u,
        Ok(None) => return Ok(None),
        Err(users::UserError::Db(e)) => return Err(e),
        Err(_) => return Ok(None),
    };
    Ok(Some(Session {
        user,
        method: Method::parse(&method),
        keep,
        expires_at,
        renewed,
    }))
}

pub fn delete(db: &Db, token: &str) -> rusqlite::Result<()> {
    db.with(|c| c.execute("DELETE FROM sessions WHERE id = ?1", [digest(token)]))
        .map(|_| ())
}

/// Ends every session of a user, except the one holding `except_token`.
pub fn delete_for_user(db: &Db, user_id: i64, except_token: Option<&str>) -> rusqlite::Result<()> {
    let keep = except_token.map(digest).unwrap_or_default();
    db.with(|c| {
        c.execute(
            "DELETE FROM sessions WHERE user_id = ?1 AND id != ?2",
            params![user_id, keep],
        )
    })
    .map(|_| ())
}

/// A short label for the Profile screen, like "Firefox on Linux".
pub fn label_from_user_agent(ua: &str) -> String {
    let browser = ["Firefox", "Edg", "Chrome", "Safari"]
        .into_iter()
        .find(|b| ua.contains(b));
    let os = ["Windows", "Android", "iPhone", "Mac OS", "Linux"]
        .into_iter()
        .find(|o| ua.contains(o));
    match (browser, os) {
        (Some(b), Some(o)) => format!("{} on {o}", if b == "Edg" { "Edge" } else { b }),
        (Some(b), None) => (if b == "Edg" { "Edge" } else { b }).to_string(),
        _ => "Unknown browser".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::users::{NewUser, Perms, create as create_user};

    fn setup() -> (Db, i64) {
        let db = Db::open_in_memory();
        let perms = Perms {
            read: true,
            write: false,
            share: false,
            admin: false,
        };
        let new = NewUser {
            email: "u@example.com".into(),
            name: "U".into(),
            password_hash: None,
            scope: "/".into(),
            perms,
        };
        let id = create_user(&db, new, 0).unwrap().id;
        (db, id)
    }

    fn rows(db: &Db) -> i64 {
        db.with(|c| c.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0)))
            .unwrap()
    }

    const T0: i64 = 1_000_000;

    #[test]
    fn token_shape() {
        let (db, id) = setup();
        let token = create(&db, id, Method::Password, false, "x", T0).unwrap();
        assert_eq!(token.len(), 43);
        assert!(
            token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
    }

    #[test]
    fn stored_as_hash() {
        let (db, id) = setup();
        let token = create(&db, id, Method::Password, false, "x", T0).unwrap();
        let stored: Vec<u8> = db
            .with(|c| c.query_row("SELECT id FROM sessions", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(stored.len(), 32);
        assert_ne!(stored, token.as_bytes());
    }

    #[test]
    fn lookup_round_trip() {
        let (db, id) = setup();
        let token = create(&db, id, Method::Password, true, "x", T0).unwrap();
        let s = lookup(&db, &token, T0 + 1).unwrap().unwrap();
        assert_eq!(
            (s.user.id, s.method, s.keep, s.renewed),
            (id, Method::Password, true, false)
        );
        assert!(lookup(&db, "not-a-token", T0).unwrap().is_none());
    }

    #[test]
    fn short_session_expires_after_12h() {
        let (db, id) = setup();
        let token = create(&db, id, Method::Password, false, "x", T0).unwrap();
        assert!(lookup(&db, &token, T0 + SHORT_SECS - 1).unwrap().is_some());
        assert!(lookup(&db, &token, T0 + SHORT_SECS + 1).unwrap().is_none());
    }

    #[test]
    fn short_session_does_not_slide() {
        let (db, id) = setup();
        let token = create(&db, id, Method::Password, false, "x", T0).unwrap();
        let s = lookup(&db, &token, T0 + SHORT_SECS - 10).unwrap().unwrap();
        assert!(!s.renewed);
        assert_eq!(s.expires_at, T0 + SHORT_SECS);
    }

    #[test]
    fn keep_session_slides() {
        let (db, id) = setup();
        let token = create(&db, id, Method::Password, true, "x", T0).unwrap();
        let later = T0 + KEEP_SECS / 2 + 1;
        let s = lookup(&db, &token, later).unwrap().unwrap();
        assert!(s.renewed);
        assert_eq!(s.expires_at, later + KEEP_SECS);
        assert!(
            lookup(&db, &token, T0 + KEEP_SECS + 10).unwrap().is_some(),
            "the slide was saved"
        );
    }

    #[test]
    fn expired_row_deleted() {
        let (db, id) = setup();
        let token = create(&db, id, Method::Password, false, "x", T0).unwrap();
        assert!(lookup(&db, &token, T0 + SHORT_SECS + 1).unwrap().is_none());
        assert_eq!(rows(&db), 0);
    }

    #[test]
    fn delete_for_user_except() {
        let (db, id) = setup();
        let keep = create(&db, id, Method::Password, false, "x", T0).unwrap();
        create(&db, id, Method::Password, false, "x", T0).unwrap();
        create(&db, id, Method::Oidc, false, "x", T0).unwrap();
        delete_for_user(&db, id, Some(&keep)).unwrap();
        assert_eq!(rows(&db), 1);
        assert!(lookup(&db, &keep, T0).unwrap().is_some());
        delete(&db, &keep).unwrap();
        assert_eq!(rows(&db), 0);
    }

    #[test]
    fn labels() {
        let ff = "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0";
        let android = "Mozilla/5.0 (Linux; Android 15) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Mobile Safari/537.36";
        assert_eq!(label_from_user_agent(ff), "Firefox on Linux");
        assert_eq!(label_from_user_agent(android), "Chrome on Android");
        assert_eq!(label_from_user_agent("curl/8.0"), "Unknown browser");
    }
}
