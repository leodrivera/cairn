//! Share links: the `shares` table. A row is only the link; whether it still works is checked on every use.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::db::Db;
use crate::fs::path::RelPath;
use crate::fs::scope::Scope;
use crate::users::{self, User};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareRow {
    pub id: String,
    pub user_id: i64,
    /// Relative to `CAIRN_ROOT`, written like a scope: `/photos/2026/trail-ridge.jpg`, `/` for the root.
    pub path: String,
    pub password_hash: Option<String>,
    pub download: bool,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub hits: i64,
}

/// 128 random bits, base64url without padding: 22 characters.
pub fn new_id() -> String {
    let mut raw = [0u8; 16];
    getrandom::fill(&mut raw).expect("system random source");
    URL_SAFE_NO_PAD.encode(raw)
}

pub fn is_id(s: &str) -> bool {
    s.len() == 22
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The expiry for a choice of 1, 7 or 30 days; `None` never expires.
pub fn expiry(days: Option<i64>, now: i64) -> Result<Option<i64>, &'static str> {
    match days {
        None => Ok(None),
        Some(d @ (1 | 7 | 30)) => Ok(Some(now + d * 86_400)),
        Some(_) => Err("Expiry must be 1, 7 or 30 days, or never."),
    }
}

/// The stored form of `rel`, a path beneath the user's `scope`.
pub fn root_path(scope: &str, rel: &RelPath) -> String {
    let base = RelPath::parse(scope).unwrap_or_else(|_| RelPath::parse("").expect("root"));
    let mut full = base;
    for seg in rel.segments() {
        full = full.join_bytes(seg).expect("segments were valid");
    }
    format!("/{full}")
}

/// The part of a stored path beneath `scope`, segment by segment, so `/photos` does not contain `/photos2`.
pub fn split_scope(stored: &str, scope: &str) -> Option<RelPath> {
    let full = RelPath::parse(stored).ok()?;
    let base = RelPath::parse(scope).ok()?;
    if !full.starts_with(&base) {
        return None;
    }
    let mut rest = RelPath::parse("").ok()?;
    for seg in &full.segments()[base.len()..] {
        rest = rest.join_bytes(seg).ok()?;
    }
    Some(rest)
}

const COLUMNS: &str =
    "id, user_id, path, password_hash, allow_download, expires_at, created_at, hits";

fn from_row(r: &Row) -> rusqlite::Result<ShareRow> {
    Ok(ShareRow {
        id: r.get(0)?,
        user_id: r.get(1)?,
        path: r.get(2)?,
        password_hash: r.get(3)?,
        download: r.get(4)?,
        expires_at: r.get(5)?,
        created_at: r.get(6)?,
        hits: r.get(7)?,
    })
}

/// Expired rows go on the next write, not on a timer.
fn purge(c: &Connection, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "DELETE FROM shares WHERE expires_at IS NOT NULL AND expires_at <= ?1",
        [now],
    )?;
    Ok(())
}

fn write<T>(
    db: &Db,
    now: i64,
    f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    db.with(|c| {
        let tx = c.unchecked_transaction()?;
        purge(&tx, now)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    })
}

pub fn insert(db: &Db, row: &ShareRow, now: i64) -> rusqlite::Result<()> {
    write(db, now, |c| {
        c.execute(
            "INSERT INTO shares (id, user_id, path, password_hash, allow_download, expires_at, created_at, hits)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                row.id,
                row.user_id,
                row.path,
                row.password_hash,
                row.download,
                row.expires_at,
                row.created_at,
                row.hits
            ],
        )?;
        Ok(())
    })
}

pub fn get(db: &Db, id: &str) -> rusqlite::Result<Option<ShareRow>> {
    let sql = format!("SELECT {COLUMNS} FROM shares WHERE id = ?1");
    db.with(|c| c.query_row(&sql, [id], from_row).optional())
}

/// One user's links, or every link with `None`; newest first, expired ones left out.
pub fn list(db: &Db, user: Option<i64>, now: i64) -> rusqlite::Result<Vec<ShareRow>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM shares WHERE (?1 IS NULL OR user_id = ?1)
         AND (expires_at IS NULL OR expires_at > ?2) ORDER BY created_at DESC, rowid DESC"
    );
    db.with(|c| {
        let mut q = c.prepare(&sql)?;
        q.query_map(params![user, now], from_row)?.collect()
    })
}

/// `None` leaves a field as it is; `Some(None)` clears the expiry or the password. False when there is no such row.
pub fn update(
    db: &Db,
    id: &str,
    expires_at: Option<Option<i64>>,
    password_hash: Option<Option<String>>,
    download: Option<bool>,
    now: i64,
) -> rusqlite::Result<bool> {
    write(db, now, |c| {
        let mut n = c.query_row("SELECT count(*) FROM shares WHERE id = ?1", [id], |r| {
            r.get::<_, i64>(0)
        })?;
        if let Some(v) = expires_at {
            n = c.execute(
                "UPDATE shares SET expires_at = ?2 WHERE id = ?1",
                params![id, v],
            )? as i64;
        }
        if let Some(v) = password_hash {
            n = c.execute(
                "UPDATE shares SET password_hash = ?2 WHERE id = ?1",
                params![id, v],
            )? as i64;
        }
        if let Some(v) = download {
            n = c.execute(
                "UPDATE shares SET allow_download = ?2 WHERE id = ?1",
                params![id, v],
            )? as i64;
        }
        Ok(n > 0)
    })
}

pub fn delete(db: &Db, id: &str, now: i64) -> rusqlite::Result<bool> {
    write(db, now, |c| {
        Ok(c.execute("DELETE FROM shares WHERE id = ?1", [id])? > 0)
    })
}

/// One more opening of the public page.
pub fn hit(db: &Db, id: &str) -> rusqlite::Result<()> {
    db.with(|c| c.execute("UPDATE shares SET hits = hits + 1 WHERE id = ?1", [id]))?;
    Ok(())
}

/// What a live link opens. A file is reached through the creator's scope; a folder is a scope of its own, so links
/// inside it cannot lead to its siblings.
pub enum Target {
    File { scope: Scope, path: RelPath },
    Folder { scope: Scope },
}

pub struct Live {
    pub row: ShareRow,
    pub creator: User,
    pub target: Target,
}

/// The link, if it still works: not expired, its creator still may share, its target still inside the creator's
/// scope and on disk. `None` for every way of being gone. Blocking.
pub fn resolve(
    db: &Db,
    root: &Path,
    data_id: Option<(u64, u64)>,
    fallback: bool,
    id: &str,
    now: i64,
) -> Option<Live> {
    let logged = |e: &dyn std::fmt::Display| tracing::error!(event = "db.error", error = %e);
    let row = get(db, id).inspect_err(|e| logged(e)).ok()??;
    if row.expires_at.is_some_and(|at| at <= now) {
        return None;
    }
    let creator = users::by_id(db, row.user_id).ok()??;
    if !creator.perms.share || !creator.perms.read {
        return None;
    }
    let rel = split_scope(&row.path, &creator.scope)?;
    let scope = Scope::new(root, &creator.scope, data_id).ok()?;
    let scope = if fallback {
        scope.with_fallback()
    } else {
        scope
    };
    let fd = scope.open(&rel, rustix::fs::OFlags::PATH).ok()?;
    let st = rustix::fs::fstat(&fd).ok()?;
    let target =
        if rustix::fs::FileType::from_raw_mode(st.st_mode) == rustix::fs::FileType::Directory {
            Target::Folder {
                scope: scope.sub(&rel).ok()?,
            }
        } else {
            Target::File { scope, path: rel }
        };
    Some(Live {
        row,
        creator,
        target,
    })
}

/// The unlock cookie of a link with a password. Scoped to the link's own path, so one link's cookie never reaches
/// another.
pub const UNLOCK_COOKIE: &str = "cairn_share";
const UNLOCK_SECS: i64 = 12 * 3600;

fn unlock_mac(id: &str, exp: i64, password_hash: &str, key: &[u8; 32]) -> ring::hmac::Tag {
    let k = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
    ring::hmac::sign(&k, format!("{id}\n{exp}\n{password_hash}").as_bytes())
}

/// Proof the visitor typed the password: the expiry and a MAC over the link, the expiry and the password hash, so
/// a new or removed password ends it.
pub fn unlock_cookie(
    id: &str,
    password_hash: &str,
    key: &[u8; 32],
    now: i64,
    https: bool,
) -> axum::http::HeaderValue {
    let exp = now + UNLOCK_SECS;
    let tag = URL_SAFE_NO_PAD.encode(unlock_mac(id, exp, password_hash, key).as_ref());
    let secure = if https { "; Secure" } else { "" };
    axum::http::HeaderValue::from_str(&format!(
        "{UNLOCK_COOKIE}={exp}.{tag}; Path=/s/{id}; HttpOnly; SameSite=Lax; Max-Age={UNLOCK_SECS}{secure}"
    ))
    .expect("cookie is ASCII")
}

pub fn read_unlock(
    headers: &axum::http::HeaderMap,
    id: &str,
    password_hash: &str,
    key: &[u8; 32],
    now: i64,
) -> bool {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .flat_map(|v| v.to_str().unwrap_or("").split(';'))
        .filter_map(|part| part.trim().strip_prefix(UNLOCK_COOKIE)?.strip_prefix('='))
        .any(|value| {
            let Some((exp, tag)) = value.split_once('.') else {
                return false;
            };
            let (Ok(exp), Ok(tag)) = (exp.parse::<i64>(), URL_SAFE_NO_PAD.decode(tag)) else {
                return false;
            };
            exp > now
                && crate::config::secret::ct_eq(
                    unlock_mac(id, exp, password_hash, key).as_ref(),
                    &tag,
                )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::users::{NewUser, Perms};

    const NOW: i64 = 1_800_000_000;

    fn user(db: &Db, email: &str) -> i64 {
        users::create(
            db,
            NewUser {
                email: email.into(),
                name: "Ana".into(),
                password_hash: None,
                scope: "/".into(),
                perms: Perms {
                    read: true,
                    write: false,
                    share: true,
                    admin: false,
                },
            },
            0,
        )
        .unwrap()
        .id
    }

    fn row(user_id: i64, path: &str, created_at: i64) -> ShareRow {
        ShareRow {
            id: new_id(),
            user_id,
            path: path.into(),
            password_hash: None,
            download: true,
            expires_at: None,
            created_at,
            hits: 0,
        }
    }

    fn count(db: &Db) -> i64 {
        db.with(|c| c.query_row("SELECT count(*) FROM shares", [], |r| r.get(0)))
            .unwrap()
    }

    #[test]
    fn ids_are_22_base64url() {
        let ids: std::collections::HashSet<String> = (0..100).map(|_| new_id()).collect();
        assert_eq!(ids.len(), 100);
        for id in &ids {
            assert_eq!(id.len(), 22, "{id}");
            assert!(is_id(id), "{id}");
        }
    }

    #[test]
    fn is_id_rejects() {
        let ok = "k7Qw3nTzAbCdEfGhIjKl-_";
        assert!(is_id(ok));
        assert!(!is_id(&ok[..21]));
        assert!(!is_id(&format!("{ok}x")));
        for bad in [
            "k7Qw3nTzAbCdEfGhIjKl+_",
            "k7Qw3nTzAbCdEfGhIjKl/_",
            "k7Qw3nTzAbCdEfGhIjKl=_",
        ] {
            assert!(!is_id(bad), "{bad}");
        }
    }

    #[test]
    fn expiry_choices() {
        assert_eq!(expiry(Some(1), NOW), Ok(Some(NOW + 86_400)));
        assert_eq!(expiry(Some(7), NOW), Ok(Some(NOW + 7 * 86_400)));
        assert_eq!(expiry(Some(30), NOW), Ok(Some(NOW + 30 * 86_400)));
        assert_eq!(expiry(None, NOW), Ok(None));
        for bad in [2, 0, -1] {
            assert_eq!(
                expiry(Some(bad), NOW),
                Err("Expiry must be 1, 7 or 30 days, or never.")
            );
        }
    }

    #[test]
    fn root_path_joins_scope() {
        let p = |s| RelPath::parse(s).unwrap();
        assert_eq!(root_path("/", &p("photos/a.jpg")), "/photos/a.jpg");
        assert_eq!(root_path("/photos", &p("2026/a.jpg")), "/photos/2026/a.jpg");
        assert_eq!(root_path("/photos", &p("")), "/photos");
        assert_eq!(root_path("/", &p("")), "/");
    }

    #[test]
    fn split_scope_segments() {
        let p = |s| RelPath::parse(s).unwrap();
        assert_eq!(split_scope("/photos/a", "/photos"), Some(p("a")));
        assert_eq!(split_scope("/photos", "/photos"), Some(p("")));
        assert_eq!(split_scope("/photos2/a", "/photos"), None);
        assert_eq!(split_scope("/docs/x", "/photos"), None);
        assert_eq!(split_scope("/photos2/a", "/"), Some(p("photos2/a")));
        assert_eq!(split_scope("/", "/"), Some(p("")));
    }

    #[test]
    fn round_trip() {
        let db = Db::open_in_memory();
        let u = user(&db, "ana@example.com");
        let mut r = row(u, "/photos/a.jpg", NOW);
        r.password_hash = Some("$argon2id$x".into());
        r.download = false;
        r.expires_at = Some(NOW + 10);
        insert(&db, &r, NOW).unwrap();
        assert_eq!(get(&db, &r.id).unwrap(), Some(r));
        assert_eq!(get(&db, "nope").unwrap(), None);
    }

    #[test]
    fn list_scoped_and_ordered() {
        let db = Db::open_in_memory();
        let (a, b) = (user(&db, "ana@example.com"), user(&db, "bo@example.com"));
        let older = row(a, "/x", NOW - 10);
        let newer = row(a, "/y", NOW);
        let other = row(b, "/z", NOW - 5);
        for r in [&older, &newer, &other] {
            insert(&db, r, NOW).unwrap();
        }
        let ids = |v: Vec<ShareRow>| v.into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(
            ids(list(&db, Some(a), NOW).unwrap()),
            vec![newer.id.clone(), older.id.clone()]
        );
        assert_eq!(
            ids(list(&db, None, NOW).unwrap()),
            vec![newer.id, other.id, older.id]
        );
    }

    #[test]
    fn list_hides_expired() {
        let db = Db::open_in_memory();
        let a = user(&db, "ana@example.com");
        let mut r = row(a, "/x", NOW - 100);
        r.expires_at = Some(NOW);
        insert(&db, &r, NOW - 100).unwrap();
        assert!(list(&db, None, NOW).unwrap().is_empty());
        assert_eq!(list(&db, None, NOW - 1).unwrap().len(), 1);
    }

    #[test]
    fn purge_on_write() {
        let db = Db::open_in_memory();
        let a = user(&db, "ana@example.com");
        let mut old = row(a, "/x", NOW - 100);
        old.expires_at = Some(NOW - 1);
        insert(&db, &old, NOW - 100).unwrap();
        assert_eq!(count(&db), 1);
        insert(&db, &row(a, "/y", NOW), NOW).unwrap();
        assert_eq!(count(&db), 1);
        assert_eq!(get(&db, &old.id).unwrap(), None);
        // Update and delete purge too.
        let mut gone = row(a, "/z", NOW);
        gone.expires_at = Some(NOW + 5);
        insert(&db, &gone, NOW).unwrap();
        update(&db, "missing", None, None, Some(true), NOW + 5).unwrap();
        assert_eq!(get(&db, &gone.id).unwrap(), None);
        let mut gone = row(a, "/w", NOW);
        gone.expires_at = Some(NOW + 5);
        insert(&db, &gone, NOW).unwrap();
        delete(&db, "missing", NOW + 5).unwrap();
        assert_eq!(get(&db, &gone.id).unwrap(), None);
    }

    #[test]
    fn hit_counts() {
        let db = Db::open_in_memory();
        let r = row(user(&db, "ana@example.com"), "/x", NOW);
        insert(&db, &r, NOW).unwrap();
        for _ in 0..3 {
            hit(&db, &r.id).unwrap();
        }
        assert_eq!(get(&db, &r.id).unwrap().unwrap().hits, 3);
    }

    #[test]
    fn cascade_on_user_delete() {
        let db = Db::open_in_memory();
        let a = user(&db, "ana@example.com");
        let r = row(a, "/x", NOW);
        insert(&db, &r, NOW).unwrap();
        users::delete(&db, a).unwrap();
        assert_eq!(get(&db, &r.id).unwrap(), None);
    }

    #[test]
    fn update_fields() {
        let db = Db::open_in_memory();
        let mut r = row(user(&db, "ana@example.com"), "/x", NOW);
        r.password_hash = Some("h".into());
        r.expires_at = Some(NOW + 100);
        insert(&db, &r, NOW).unwrap();
        assert!(update(&db, &r.id, None, None, Some(false), NOW).unwrap());
        assert!(!get(&db, &r.id).unwrap().unwrap().download);
        assert!(update(&db, &r.id, Some(Some(NOW + 50)), None, None, NOW).unwrap());
        assert_eq!(get(&db, &r.id).unwrap().unwrap().expires_at, Some(NOW + 50));
        assert!(update(&db, &r.id, None, Some(Some("h2".into())), None, NOW).unwrap());
        assert_eq!(
            get(&db, &r.id).unwrap().unwrap().password_hash.as_deref(),
            Some("h2")
        );
        assert!(update(&db, &r.id, Some(None), Some(None), None, NOW).unwrap());
        let got = get(&db, &r.id).unwrap().unwrap();
        assert_eq!((got.expires_at, got.password_hash), (None, None));
        assert!(!update(&db, "missing", None, None, Some(true), NOW).unwrap());
        assert!(delete(&db, &r.id, NOW).unwrap());
        assert!(!delete(&db, &r.id, NOW).unwrap());
    }
    const KEY: [u8; 32] = [7; 32];
    const ID: &str = "k7Qw3nTzAbCdEfGhIjKl-_";

    fn cookie_headers(c: &axum::http::HeaderValue) -> axum::http::HeaderMap {
        let pair = c.to_str().unwrap().split(';').next().unwrap().to_string();
        let mut h = axum::http::HeaderMap::new();
        h.insert("cookie", format!("other=1; {pair}").parse().unwrap());
        h
    }

    #[test]
    fn cookie_round_trip() {
        let c = unlock_cookie(ID, "hash", &KEY, NOW, false);
        let text = c.to_str().unwrap();
        assert!(text.starts_with("cairn_share="), "{text}");
        assert!(text.contains(&format!("; Path=/s/{ID}")), "{text}");
        assert!(
            text.contains("; HttpOnly; SameSite=Lax; Max-Age=43200"),
            "{text}"
        );
        assert!(!text.contains("Secure"));
        assert!(
            unlock_cookie(ID, "hash", &KEY, NOW, true)
                .to_str()
                .unwrap()
                .ends_with("; Secure")
        );
        assert!(read_unlock(
            &cookie_headers(&c),
            ID,
            "hash",
            &KEY,
            NOW + 43_199
        ));
    }

    #[test]
    fn cookie_expires() {
        let c = unlock_cookie(ID, "hash", &KEY, NOW, false);
        assert!(!read_unlock(
            &cookie_headers(&c),
            ID,
            "hash",
            &KEY,
            NOW + 43_200
        ));
    }

    #[test]
    fn cookie_bound_to_hash() {
        let c = unlock_cookie(ID, "hash", &KEY, NOW, false);
        assert!(!read_unlock(&cookie_headers(&c), ID, "other", &KEY, NOW));
        assert!(!read_unlock(&cookie_headers(&c), ID, "hash", &[8; 32], NOW));
    }

    #[test]
    fn cookie_bound_to_id() {
        let c = unlock_cookie(ID, "hash", &KEY, NOW, false);
        assert!(!read_unlock(
            &cookie_headers(&c),
            "Zt6yQn1wAbCdEfGhIjKl-_",
            "hash",
            &KEY,
            NOW
        ));
    }

    #[test]
    fn cookie_tampered() {
        let c = unlock_cookie(ID, "hash", &KEY, NOW, false);
        let pair = c.to_str().unwrap().split(';').next().unwrap().to_string();
        let value = pair.strip_prefix("cairn_share=").unwrap();
        let (exp, tag) = value.split_once('.').unwrap();
        let later = format!("cairn_share={}.{tag}", exp.parse::<i64>().unwrap() + 1000);
        let flipped = {
            let mut t = tag.to_string();
            let last = if t.ends_with('A') { 'B' } else { 'A' };
            t.pop();
            t.push(last);
            format!("cairn_share={exp}.{t}")
        };
        for bad in [
            later,
            flipped,
            "cairn_share=".into(),
            "cairn_share=x.y".into(),
        ] {
            let mut h = axum::http::HeaderMap::new();
            h.insert("cookie", bad.parse().unwrap());
            assert!(!read_unlock(&h, ID, "hash", &KEY, NOW), "{bad}");
        }
    }
}
