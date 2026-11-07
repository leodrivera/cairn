//! Users: e-mail, name, optional password, scope and permissions.

use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::db::Db;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Perms {
    pub read: bool,
    pub write: bool,
    pub share: bool,
    pub admin: bool,
}

impl Perms {
    /// `write` and `share` both need `read`.
    fn normalized(self) -> Perms {
        Perms {
            read: self.read || self.write || self.share,
            ..self
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: i64,
    pub email: String,
    pub name: String,
    pub scope: String,
    #[serde(rename = "permissions")]
    pub perms: Perms,
    pub lang: Option<String>,
    pub theme: Option<String>,
    pub view: Option<String>,
    pub has_password: bool,
}

pub struct NewUser {
    pub email: String,
    pub name: String,
    pub password_hash: Option<String>,
    pub scope: String,
    pub perms: Perms,
}

/// `None` leaves a field as it is. For the preferences, `Some(None)` clears it.
#[derive(Default)]
pub struct UserPatch {
    pub email: Option<String>,
    pub name: Option<String>,
    pub scope: Option<String>,
    pub perms: Option<Perms>,
    pub lang: Option<Option<String>>,
    pub theme: Option<Option<String>>,
    pub view: Option<Option<String>>,
}

#[derive(Debug)]
pub enum UserError {
    Invalid(&'static str),
    EmailTaken,
    NotFound,
    LastAdmin,
    Db(rusqlite::Error),
}

impl UserError {
    pub fn message(&self) -> String {
        match self {
            UserError::Invalid(m) => (*m).to_string(),
            UserError::EmailTaken => "Another user already has this e-mail.".into(),
            UserError::NotFound => "No such user.".into(),
            UserError::LastAdmin => {
                "Cairn needs at least one admin. Make another user an admin first.".into()
            }
            UserError::Db(_) => {
                "Something went wrong on the server. The log has the details.".into()
            }
        }
    }
}

impl From<rusqlite::Error> for UserError {
    fn from(e: rusqlite::Error) -> Self {
        match &e {
            // Only the UNIQUE index on users.email means "taken"; any other constraint is a real error.
            rusqlite::Error::SqliteFailure(f, Some(msg))
                if f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                    && msg.contains("users.email") =>
            {
                UserError::EmailTaken
            }
            _ => UserError::Db(e),
        }
    }
}

const BAD_EMAIL: &str = "Enter a valid e-mail address.";
const BAD_SCOPE: &str = "The scope must be a folder path like /photos.";

/// Trimmed and lower-cased; `None` unless it is one plausible address.
pub fn normalize_email(raw: &str) -> Option<String> {
    let email = raw.trim().to_lowercase();
    let (local, domain) = email.split_once('@')?;
    let ok = email.len() <= 254
        && !local.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !email
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || ",;<>".contains(c));
    ok.then_some(email)
}

/// Whether `normalize_email` changes `raw` only by trimming ASCII spaces and lower-casing ASCII letters.
pub fn normalizes_plainly(raw: &str) -> bool {
    normalize_email(raw).is_some_and(|email| email == raw.trim_ascii().to_ascii_lowercase())
}

fn clean_name(raw: &str) -> Result<String, UserError> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(UserError::Invalid("Name is required."));
    }
    Ok(name.to_string())
}

fn clean_scope(raw: &str) -> Result<String, UserError> {
    if !raw.starts_with('/') || raw.len() > 4096 || raw.contains('\0') {
        return Err(UserError::Invalid(BAD_SCOPE));
    }
    if raw == "/" {
        return Ok("/".into());
    }
    let trimmed = raw.strip_suffix('/').unwrap_or(raw);
    if trimmed[1..]
        .split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err(UserError::Invalid(BAD_SCOPE));
    }
    Ok(trimmed.to_string())
}

fn check_choice(
    value: &Option<String>,
    allowed: &[&str],
    message: &'static str,
) -> Result<(), UserError> {
    match value {
        Some(v) if !allowed.contains(&v.as_str()) => Err(UserError::Invalid(message)),
        _ => Ok(()),
    }
}

const COLUMNS: &str = "id, email, name, scope, perm_read, perm_write, perm_share, perm_admin, lang, theme, view, \
                       password_hash IS NOT NULL, password_hash";

fn from_row(r: &Row) -> rusqlite::Result<(User, Option<String>)> {
    Ok((
        User {
            id: r.get(0)?,
            email: r.get(1)?,
            name: r.get(2)?,
            scope: r.get(3)?,
            perms: Perms {
                read: r.get(4)?,
                write: r.get(5)?,
                share: r.get(6)?,
                admin: r.get(7)?,
            },
            lang: r.get(8)?,
            theme: r.get(9)?,
            view: r.get(10)?,
            has_password: r.get(11)?,
        },
        r.get(12)?,
    ))
}

pub fn create(db: &Db, new: NewUser, now: i64) -> Result<User, UserError> {
    let email = normalize_email(&new.email).ok_or(UserError::Invalid(BAD_EMAIL))?;
    let name = clean_name(&new.name)?;
    let scope = clean_scope(&new.scope)?;
    let p = new.perms.normalized();
    let id = db.with(|c| {
        c.execute(
            "INSERT INTO users (email, name, password_hash, scope, perm_read, perm_write, perm_share, perm_admin, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![email, name, new.password_hash, scope, p.read, p.write, p.share, p.admin, now],
        )?;
        Ok(c.last_insert_rowid())
    })?;
    by_id(db, id)?.ok_or(UserError::NotFound)
}

pub fn by_id(db: &Db, id: i64) -> Result<Option<User>, UserError> {
    let sql = format!("SELECT {COLUMNS} FROM users WHERE id = ?1");
    Ok(db
        .with(|c| c.query_row(&sql, [id], from_row).optional())?
        .map(|(u, _)| u))
}

/// The user and their password hash. `email` must already be normalized.
pub fn by_email(db: &Db, email: &str) -> Result<Option<(User, Option<String>)>, UserError> {
    let sql = format!("SELECT {COLUMNS} FROM users WHERE email = ?1");
    Ok(db.with(|c| c.query_row(&sql, [email], from_row).optional())?)
}

pub fn list(db: &Db) -> Result<Vec<User>, UserError> {
    let sql = format!("SELECT {COLUMNS} FROM users ORDER BY email");
    Ok(db.with(|c| {
        let mut q = c.prepare(&sql)?;
        q.query_map([], from_row)?
            .map(|r| r.map(|(u, _)| u))
            .collect()
    })?)
}

pub fn count(db: &Db) -> Result<i64, UserError> {
    Ok(db.with(|c| c.query_row("SELECT count(*) FROM users", [], |r| r.get(0)))?)
}

#[cfg(test)]
fn admins(db: &Db) -> Result<i64, UserError> {
    Ok(db.with(|c| {
        c.query_row("SELECT count(*) FROM users WHERE perm_admin = 1", [], |r| {
            r.get(0)
        })
    })?)
}

pub fn update(db: &Db, id: i64, patch: UserPatch) -> Result<User, UserError> {
    let email = match &patch.email {
        Some(e) => Some(normalize_email(e).ok_or(UserError::Invalid(BAD_EMAIL))?),
        None => None,
    };
    let name = patch.name.as_deref().map(clean_name).transpose()?;
    let scope = patch.scope.as_deref().map(clean_scope).transpose()?;
    let perms = patch.perms.map(|p| p.normalized());
    if let Some(lang) = &patch.lang {
        check_choice(lang, &["en", "pt-BR"], "Language must be en or pt-BR.")?;
    }
    if let Some(theme) = &patch.theme {
        check_choice(theme, &["dark", "light"], "Theme must be dark or light.")?;
    }
    if let Some(view) = &patch.view {
        check_choice(view, &["list", "gallery"], "View must be list or gallery.")?;
    }
    let select = format!("SELECT {COLUMNS} FROM users WHERE id = ?1");
    let outcome = db.with(|c| {
        // The row is read under the write lock: a field the patch does not name keeps its current value, even when
        // another change committed a moment ago.
        immediate(c, || {
            let Some((mut u, _)) = c.query_row(&select, [id], from_row).optional()? else {
                return Ok(Outcome::NotFound);
            };
            if let Some(v) = email {
                u.email = v;
            }
            if let Some(v) = name {
                u.name = v;
            }
            if let Some(v) = scope {
                u.scope = v;
            }
            if let Some(v) = perms {
                u.perms = v;
            }
            if let Some(v) = patch.lang {
                u.lang = v;
            }
            if let Some(v) = patch.theme {
                u.theme = v;
            }
            if let Some(v) = patch.view {
                u.view = v;
            }
            let p = u.perms;
            if !p.admin && removes_last_admin(c, id)? {
                return Ok(Outcome::LastAdmin);
            }
            c.execute(
                "UPDATE users SET email = ?2, name = ?3, scope = ?4, perm_read = ?5, perm_write = ?6, perm_share = ?7,
                 perm_admin = ?8, lang = ?9, theme = ?10, view = ?11 WHERE id = ?1",
                params![id, u.email, u.name, u.scope, p.read, p.write, p.share, p.admin, u.lang, u.theme, u.view],
            )?;
            Ok(Outcome::Done)
        })
    })?;
    match outcome {
        Outcome::Done => by_id(db, id)?.ok_or(UserError::NotFound),
        Outcome::NotFound => Err(UserError::NotFound),
        Outcome::LastAdmin => Err(UserError::LastAdmin),
    }
}

pub fn set_password(db: &Db, id: i64, hash: Option<String>) -> Result<(), UserError> {
    let n = db.with(|c| {
        c.execute(
            "UPDATE users SET password_hash = ?2 WHERE id = ?1",
            params![id, hash],
        )
    })?;
    if n == 0 {
        Err(UserError::NotFound)
    } else {
        Ok(())
    }
}

/// Sets the password only if `verified` is still the stored hash, in one statement. `false` when it no longer is: a
/// reset or a removal landed during the slow check, and it stands.
pub fn replace_password(db: &Db, id: i64, verified: &str, hash: &str) -> Result<bool, UserError> {
    let n = db.with(|c| {
        c.execute(
            "UPDATE users SET password_hash = ?3 WHERE id = ?1 AND password_hash = ?2",
            params![id, verified, hash],
        )
    })?;
    Ok(n == 1)
}

pub fn delete(db: &Db, id: i64) -> Result<(), UserError> {
    let outcome = db.with(|c| {
        immediate(c, || {
            if removes_last_admin(c, id)? {
                return Ok(Outcome::LastAdmin);
            }
            let n = c.execute("DELETE FROM users WHERE id = ?1", [id])?;
            Ok(if n == 0 {
                Outcome::NotFound
            } else {
                Outcome::Done
            })
        })
    })?;
    match outcome {
        Outcome::Done => Ok(()),
        Outcome::NotFound => Err(UserError::NotFound),
        Outcome::LastAdmin => Err(UserError::LastAdmin),
    }
}

#[derive(PartialEq, Eq)]
enum Outcome {
    Done,
    NotFound,
    LastAdmin,
}

/// True when `id` is an admin and the only one, so taking it away would leave none.
fn removes_last_admin(c: &rusqlite::Connection, id: i64) -> rusqlite::Result<bool> {
    c.query_row(
        "SELECT perm_admin = 1 AND (SELECT count(*) FROM users WHERE perm_admin = 1) <= 1 FROM users WHERE id = ?1",
        [id],
        |r| r.get(0),
    )
    .optional()
    .map(|v| v.unwrap_or(false))
}

/// Runs `f` in a `BEGIN IMMEDIATE` transaction: the check and the write it guards happen under one write lock,
/// also against another process (`cairn user`) on the same file.
fn immediate<T>(
    c: &rusqlite::Connection,
    f: impl FnOnce() -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    c.execute_batch("BEGIN IMMEDIATE")?;
    match f() {
        Ok(v) => {
            c.execute_batch("COMMIT")?;
            Ok(v)
        }
        Err(e) => {
            let _ = c.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Perms = Perms {
        read: true,
        write: true,
        share: true,
        admin: true,
    };
    const READ: Perms = Perms {
        read: true,
        write: false,
        share: false,
        admin: false,
    };

    fn new(email: &str, perms: Perms) -> NewUser {
        NewUser {
            email: email.into(),
            name: "Someone".into(),
            password_hash: None,
            scope: "/".into(),
            perms,
        }
    }

    #[test]
    fn create_and_find() {
        let db = Db::open_in_memory();
        let u = create(&db, new("Ana@Example.com", READ), 0).unwrap();
        assert_eq!(u.email, "ana@example.com");
        let email = normalize_email(" ANA@example.com ").unwrap();
        let (found, hash) = by_email(&db, &email).unwrap().unwrap();
        assert_eq!((found.id, hash), (u.id, None));
        assert!(!found.has_password);
    }

    #[test]
    fn email_rules() {
        assert_eq!(
            normalize_email("  Bo@Example.COM "),
            Some("bo@example.com".into())
        );
        for bad in [
            "",
            "bo",
            "bo@",
            "@example.com",
            "bo@example",
            "b o@example.com",
            "a@b@example.com",
        ] {
            assert_eq!(normalize_email(bad), None, "{bad:?}");
        }
        assert_eq!(
            normalize_email(&format!("{}@example.com", "a".repeat(250))),
            None
        );
    }

    #[test]
    fn email_taken() {
        let db = Db::open_in_memory();
        create(&db, new("ana@example.com", READ), 0).unwrap();
        assert!(matches!(
            create(&db, new("ANA@example.com", READ), 0),
            Err(UserError::EmailTaken)
        ));
    }

    #[test]
    fn write_implies_read() {
        let db = Db::open_in_memory();
        let perms = Perms {
            read: false,
            write: true,
            share: false,
            admin: false,
        };
        assert!(
            create(&db, new("w@example.com", perms), 0)
                .unwrap()
                .perms
                .read
        );
        let perms = Perms {
            read: false,
            write: false,
            share: true,
            admin: false,
        };
        assert!(
            create(&db, new("s@example.com", perms), 0)
                .unwrap()
                .perms
                .read
        );
    }

    #[test]
    fn scope_rules() {
        let db = Db::open_in_memory();
        for (i, (given, stored)) in [("/", "/"), ("/photos", "/photos"), ("/a/b/", "/a/b")]
            .into_iter()
            .enumerate()
        {
            let mut n = new(&format!("u{i}@example.com"), READ);
            n.scope = given.into();
            assert_eq!(create(&db, n, 0).unwrap().scope, stored);
        }
        for bad in ["photos", "/a/../b", "/a//b", "/./a", "/a\0b"] {
            let mut n = new("bad@example.com", READ);
            n.scope = bad.into();
            assert!(
                matches!(create(&db, n, 0), Err(UserError::Invalid(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn update_preferences() {
        let db = Db::open_in_memory();
        let u = create(&db, new("p@example.com", READ), 0).unwrap();
        let u = update(
            &db,
            u.id,
            UserPatch {
                theme: Some(Some("light".into())),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(u.theme.as_deref(), Some("light"));
        let u = update(
            &db,
            u.id,
            UserPatch {
                theme: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(u.theme, None);
        let bad = update(
            &db,
            u.id,
            UserPatch {
                theme: Some(Some("blue".into())),
                ..Default::default()
            },
        );
        assert!(matches!(bad, Err(UserError::Invalid(_))));
    }

    #[test]
    fn profile_save_keeps_a_concurrent_admin_change() {
        let db = Db::open_in_memory();
        create(&db, new("admin@example.com", ALL), 0).unwrap();
        let id = create(&db, new("ana@example.com", ALL), 0).unwrap().id;
        for _ in 0..2000 {
            let restore = UserPatch {
                perms: Some(ALL),
                ..UserPatch::default()
            };
            update(&db, id, restore).unwrap();
            let done = std::sync::atomic::AtomicBool::new(false);
            std::thread::scope(|s| {
                s.spawn(|| {
                    while !done.load(std::sync::atomic::Ordering::Relaxed) {
                        let save = UserPatch {
                            theme: Some(Some("dark".into())),
                            ..UserPatch::default()
                        };
                        update(&db, id, save).unwrap();
                    }
                });
                std::thread::yield_now();
                let change = UserPatch {
                    perms: Some(READ),
                    ..UserPatch::default()
                };
                update(&db, id, change).unwrap();
                done.store(true, std::sync::atomic::Ordering::Relaxed);
            });
            assert!(!by_id(&db, id).unwrap().unwrap().perms.admin);
        }
    }

    #[test]
    fn last_admin_cannot_be_deleted() {
        let db = Db::open_in_memory();
        let admin = create(&db, new("admin@example.com", ALL), 0).unwrap();
        assert!(matches!(delete(&db, admin.id), Err(UserError::LastAdmin)));
    }

    #[test]
    fn last_admin_cannot_lose_admin() {
        let db = Db::open_in_memory();
        let admin = create(&db, new("admin@example.com", ALL), 0).unwrap();
        let demote = UserPatch {
            perms: Some(Perms {
                admin: false,
                ..ALL
            }),
            ..Default::default()
        };
        assert!(matches!(
            update(&db, admin.id, demote),
            Err(UserError::LastAdmin)
        ));
    }

    #[test]
    fn second_admin_allows_it() {
        let db = Db::open_in_memory();
        let a = create(&db, new("a@example.com", ALL), 0).unwrap();
        create(&db, new("b@example.com", ALL), 0).unwrap();
        delete(&db, a.id).unwrap();
        assert_eq!(count(&db).unwrap(), 1);
    }

    #[test]
    fn delete_cascades_sessions() {
        let db = Db::open_in_memory();
        create(&db, new("admin@example.com", ALL), 0).unwrap();
        let u = create(&db, new("u@example.com", READ), 0).unwrap();
        db.with(|c| {
            c.execute(
                "INSERT INTO sessions (id, user_id, method, created_at, expires_at) VALUES (x'01', ?1, 'password', 0, 9)",
                [u.id],
            )
        })
        .unwrap();
        delete(&db, u.id).unwrap();
        let n: i64 = db
            .with(|c| c.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn last_admin_survives_concurrent_deletes() {
        let dir = tempfile::tempdir().unwrap();
        for round in 0..100 {
            let path = dir.path().join(format!("r{round}.db"));
            let setup = Db::open(&path).unwrap();
            let a = create(&setup, new("a@example.com", ALL), 0).unwrap().id;
            let b = create(&setup, new("b@example.com", ALL), 0).unwrap().id;
            drop(setup);
            // Two connections, like the server and `cairn user delete` at once.
            let (one, two) = (Db::open(&path).unwrap(), Db::open(&path).unwrap());
            let gate = std::sync::Barrier::new(2);
            std::thread::scope(|s| {
                s.spawn(|| {
                    gate.wait();
                    let _ = delete(&one, a);
                });
                s.spawn(|| {
                    gate.wait();
                    let _ = delete(&two, b);
                });
            });
            assert!(admins(&one).unwrap() >= 1, "round {round}: no admin left");
        }
    }

    #[test]
    fn other_constraints_are_not_email_taken() {
        let db = Db::open_in_memory();
        let err = db
            .with(|c| {
                c.execute(
                    "INSERT INTO sessions (id, user_id, method, created_at, expires_at) VALUES (x'02', 999, 'password', 0, 1)",
                    [],
                )
            })
            .unwrap_err();
        assert!(matches!(UserError::from(err), UserError::Db(_)));
    }
}
