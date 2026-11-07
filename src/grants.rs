//! Grants: the `grants` table. A row lets one user open one file or folder of another; whether it still works is
//! checked on every use, like a share link.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::db::Db;
use crate::fs::path::RelPath;
use crate::fs::scope::{Reserved, Scope};
use crate::shares::split_scope;
use crate::users::{self, User};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRow {
    pub id: String,
    pub owner_id: i64,
    pub recipient_id: i64,
    /// Relative to `CAIRN_ROOT`, written like a scope.
    pub path: String,
    pub write: bool,
    pub expires_at: Option<i64>,
    pub created_at: i64,
}

const COLUMNS: &str = "id, owner_id, recipient_id, path, write, expires_at, created_at";

fn from_row(r: &Row) -> rusqlite::Result<GrantRow> {
    Ok(GrantRow {
        id: r.get(0)?,
        owner_id: r.get(1)?,
        recipient_id: r.get(2)?,
        path: r.get(3)?,
        write: r.get(4)?,
        expires_at: r.get(5)?,
        created_at: r.get(6)?,
    })
}

/// Expired rows go on the next write, not on a timer.
fn purge(c: &Connection, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "DELETE FROM grants WHERE expires_at IS NOT NULL AND expires_at <= ?1",
        [now],
    )?;
    Ok(())
}

/// Grants `path` again or for the first time; a second grant of the same thing changes permission and expiry.
/// Takes a connection so a caller can hold several in one transaction.
pub fn upsert(
    c: &Connection,
    owner: i64,
    recipient: i64,
    path: &str,
    write: bool,
    expires_at: Option<i64>,
    now: i64,
) -> rusqlite::Result<GrantRow> {
    purge(c, now)?;
    c.execute(
        "INSERT INTO grants (id, owner_id, recipient_id, path, write, expires_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (owner_id, recipient_id, path)
         DO UPDATE SET write = excluded.write, expires_at = excluded.expires_at",
        params![
            crate::shares::new_id(),
            owner,
            recipient,
            path,
            write,
            expires_at,
            now
        ],
    )?;
    c.query_row(
        &format!(
            "SELECT {COLUMNS} FROM grants WHERE owner_id = ?1 AND recipient_id = ?2 AND path = ?3"
        ),
        params![owner, recipient, path],
        from_row,
    )
}

pub fn get(db: &Db, id: &str) -> rusqlite::Result<Option<GrantRow>> {
    db.with(|c| {
        c.query_row(
            &format!("SELECT {COLUMNS} FROM grants WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()
    })
}

fn query(db: &Db, filter: &str, who: Option<i64>, now: i64) -> rusqlite::Result<Vec<GrantRow>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM grants WHERE (?1 IS NULL OR {filter} = ?1)
         AND (expires_at IS NULL OR expires_at > ?2) ORDER BY created_at DESC, rowid DESC"
    );
    db.with(|c| {
        let mut q = c.prepare(&sql)?;
        q.query_map(params![who, now], from_row)?.collect()
    })
}

/// Grants one owner gave, or every grant with `None`; newest first, expired ones left out.
pub fn list_given(db: &Db, owner: Option<i64>, now: i64) -> rusqlite::Result<Vec<GrantRow>> {
    query(db, "owner_id", owner, now)
}

/// Grants one person received, expired ones left out (the caller checks the rest with `resolve`).
pub fn list_received(db: &Db, recipient: i64, now: i64) -> rusqlite::Result<Vec<GrantRow>> {
    query(db, "recipient_id", Some(recipient), now)
}

/// `None` leaves a field as it is. False when there is no such row.
pub fn update(
    db: &Db,
    id: &str,
    write: Option<bool>,
    expires_at: Option<Option<i64>>,
    now: i64,
) -> rusqlite::Result<bool> {
    db.with(|c| {
        let tx = c.unchecked_transaction()?;
        purge(&tx, now)?;
        let mut n = tx.query_row("SELECT count(*) FROM grants WHERE id = ?1", [id], |r| {
            r.get::<_, i64>(0)
        })?;
        if let Some(w) = write {
            n = tx.execute("UPDATE grants SET write = ?2 WHERE id = ?1", params![id, w])? as i64;
        }
        if let Some(e) = expires_at {
            n = tx.execute(
                "UPDATE grants SET expires_at = ?2 WHERE id = ?1",
                params![id, e],
            )? as i64;
        }
        tx.commit()?;
        Ok(n > 0)
    })
}

pub fn delete(db: &Db, id: &str, now: i64) -> rusqlite::Result<bool> {
    db.with(|c| {
        purge(c, now)?;
        Ok(c.execute("DELETE FROM grants WHERE id = ?1", [id])? > 0)
    })
}

/// What a live grant opens. A folder is a scope of its own; a file is reached through its parent folder as a scope,
/// limited to that one name. `base` is the stored path of the scope's own folder.
pub enum GrantTarget {
    Folder {
        scope: Scope,
        base: String,
    },
    File {
        scope: Scope,
        name: RelPath,
        base: String,
    },
}

pub struct LiveGrant {
    pub row: GrantRow,
    pub owner: User,
    /// The grant says write and the owner may write there now.
    pub write: bool,
    pub target: GrantTarget,
}

impl LiveGrant {
    pub fn is_folder(&self) -> bool {
        matches!(self.target, GrantTarget::Folder { .. })
    }
}

/// The grant, if it works for `recipient` now: not expired, its owner still may share and read, its path still in
/// the owner's scope and on disk. `None` for every way of being gone. Blocking.
pub fn resolve(
    db: &Db,
    root: &Path,
    reserved: Reserved,
    fallback: bool,
    id: &str,
    recipient: i64,
    now: i64,
) -> Option<LiveGrant> {
    let row = get(db, id)
        .inspect_err(|e| tracing::error!(event = "db.error", error = %e))
        .ok()??;
    live(db, root, reserved, fallback, row, recipient, now)
}

/// `resolve` for a row already read.
pub fn live(
    db: &Db,
    root: &Path,
    reserved: Reserved,
    fallback: bool,
    row: GrantRow,
    recipient: i64,
    now: i64,
) -> Option<LiveGrant> {
    if row.recipient_id != recipient || row.expires_at.is_some_and(|at| at <= now) {
        return None;
    }
    let owner = users::by_id(db, row.owner_id).ok()??;
    if !owner.perms.share || !owner.perms.read {
        return None;
    }
    let rel = split_scope(&row.path, &owner.scope)?;
    let scope = Scope::new(root, &owner.scope, reserved).ok()?;
    let scope = if fallback {
        scope.with_fallback()
    } else {
        scope
    }
    .no_links();
    let fd = scope.open(&rel, rustix::fs::OFlags::PATH).ok()?;
    let st = rustix::fs::fstat(&fd).ok()?;
    let folder = rustix::fs::FileType::from_raw_mode(st.st_mode) == rustix::fs::FileType::Directory;
    let target = if folder {
        GrantTarget::Folder {
            scope: scope.sub(&rel).ok()?.follow_links(),
            base: row.path.clone(),
        }
    } else {
        let parent = rel.parent()?;
        GrantTarget::File {
            scope: scope.sub(&parent).ok()?,
            name: RelPath::parse("")
                .ok()?
                .join_bytes(rel.name_bytes()?)
                .ok()?,
            base: crate::shares::root_path(&owner.scope, &parent),
        }
    };
    Some(LiveGrant {
        write: row.write && owner.perms.write,
        row,
        owner,
        target,
    })
}
