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
            rusqlite::Error::SqliteFailure(f, _)
                if f.code == rusqlite::ErrorCode::ConstraintViolation =>
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

fn admins(db: &Db) -> Result<i64, UserError> {
    Ok(db.with(|c| {
        c.query_row("SELECT count(*) FROM users WHERE perm_admin = 1", [], |r| {
            r.get(0)
        })
    })?)
}

pub fn update(db: &Db, id: i64, patch: UserPatch) -> Result<User, UserError> {
    let mut u = by_id(db, id)?.ok_or(UserError::NotFound)?;
    if let Some(email) = &patch.email {
        u.email = normalize_email(email).ok_or(UserError::Invalid(BAD_EMAIL))?;
    }
    if let Some(name) = &patch.name {
        u.name = clean_name(name)?;
    }
    if let Some(scope) = &patch.scope {
        u.scope = clean_scope(scope)?;
    }
    if let Some(perms) = patch.perms {
        let perms = perms.normalized();
        if u.perms.admin && !perms.admin && admins(db)? <= 1 {
            return Err(UserError::LastAdmin);
        }
        u.perms = perms;
    }
    if let Some(lang) = patch.lang {
        check_choice(&lang, &["en", "pt-BR"], "Language must be en or pt-BR.")?;
        u.lang = lang;
    }
    if let Some(theme) = patch.theme {
        check_choice(&theme, &["dark", "light"], "Theme must be dark or light.")?;
        u.theme = theme;
    }
    if let Some(view) = patch.view {
        check_choice(&view, &["list", "gallery"], "View must be list or gallery.")?;
        u.view = view;
    }
    let p = u.perms;
    db.with(|c| {
        c.execute(
            "UPDATE users SET email = ?2, name = ?3, scope = ?4, perm_read = ?5, perm_write = ?6, perm_share = ?7,
             perm_admin = ?8, lang = ?9, theme = ?10, view = ?11 WHERE id = ?1",
            params![id, u.email, u.name, u.scope, p.read, p.write, p.share, p.admin, u.lang, u.theme, u.view],
        )
    })?;
    by_id(db, id)?.ok_or(UserError::NotFound)
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

pub fn delete(db: &Db, id: i64) -> Result<(), UserError> {
    let u = by_id(db, id)?.ok_or(UserError::NotFound)?;
    if u.perms.admin && admins(db)? <= 1 {
        return Err(UserError::LastAdmin);
    }
    db.with(|c| c.execute("DELETE FROM users WHERE id = ?1", [id]))?;
    Ok(())
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
}
