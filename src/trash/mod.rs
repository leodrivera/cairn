//! The trash: deleted items wait in a `.cairn-trash` folder at the top of their mount until their date, a restore
//! or a permanent delete. The `trash` table holds one row per entry; nothing else about files is kept.

pub mod folder;
pub mod ops;

use rusqlite::{OptionalExtension, params};

use crate::config::Config;
use crate::db::Db;
use crate::http::AppState;

/// Days an entry is kept when nothing is set.
pub const DEFAULT_DAYS: u32 = 30;
/// Most days `CAIRN_TRASH_DAYS` or the setting take.
pub const MAX_DAYS: u32 = 3650;

/// The token our trash folders' marker names; made on first use and kept in `settings`.
pub fn marker_token(db: &Db) -> rusqlite::Result<String> {
    db.with(|c| {
        if let Some(t) = c
            .query_row(
                "SELECT value FROM settings WHERE key = 'trash_marker'",
                [],
                |r| r.get(0),
            )
            .optional()?
        {
            return Ok(t);
        }
        let token = crate::shares::new_id();
        c.execute(
            "INSERT INTO settings (key, value) VALUES ('trash_marker', ?1)",
            params![token],
        )?;
        Ok(token)
    })
}

/// The token, if one was ever made; reading never makes one.
pub fn existing_token(db: &Db) -> Option<String> {
    db.with(|c| {
        c.query_row(
            "SELECT value FROM settings WHERE key = 'trash_marker'",
            [],
            |r| r.get(0),
        )
        .optional()
    })
    .ok()
    .flatten()
}

/// Days new entries are kept, and whether `CAIRN_TRASH_DAYS` decided it.
pub fn days(config: &Config, db: &Db) -> (u32, bool) {
    if let Some(d) = config.trash_days {
        return (d, true);
    }
    let stored = crate::settings::load(db).ok().and_then(|s| s.trash_days);
    (stored.unwrap_or(DEFAULT_DAYS), false)
}

/// Whether deletes go to the trash at all.
pub fn on(state: &AppState) -> bool {
    days(&state.config, &state.db).0 > 0
}

/// One trash entry, as the `trash` table holds it.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    /// The trash folder holding it, relative to the root: `/.cairn-trash`, `/photos/.cairn-trash`.
    pub trash_dir: String,
    /// Where it was, relative to the root.
    pub path: String,
    pub name: Vec<u8>,
    /// `file` or `folder`.
    pub kind: String,
    pub size: i64,
    pub deleted_by: Option<i64>,
    pub deleted_at: i64,
    pub expires_at: i64,
}

const COLUMNS: &str = "id, trash_dir, path, name, kind, size, deleted_by, deleted_at, expires_at";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: r.get(0)?,
        trash_dir: r.get(1)?,
        path: r.get(2)?,
        name: r.get(3)?,
        kind: r.get(4)?,
        size: r.get(5)?,
        deleted_by: r.get(6)?,
        deleted_at: r.get(7)?,
        expires_at: r.get(8)?,
    })
}

pub fn insert(db: &Db, e: &Entry) -> rusqlite::Result<()> {
    db.with(|c| {
        c.execute(
            &format!("INSERT INTO trash ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"),
            params![
                e.id,
                e.trash_dir,
                e.path,
                e.name,
                e.kind,
                e.size,
                e.deleted_by,
                e.deleted_at,
                e.expires_at
            ],
        )?;
        Ok(())
    })
}

pub fn get(db: &Db, id: &str) -> rusqlite::Result<Option<Entry>> {
    db.with(|c| {
        c.query_row(
            &format!("SELECT {COLUMNS} FROM trash WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()
    })
}

/// Every entry, newest first.
pub fn all(db: &Db) -> rusqlite::Result<Vec<Entry>> {
    db.with(|c| {
        let mut q = c.prepare(&format!(
            "SELECT {COLUMNS} FROM trash ORDER BY deleted_at DESC, rowid DESC"
        ))?;
        q.query_map([], from_row)?.collect()
    })
}

pub fn delete_row(db: &Db, id: &str) -> rusqlite::Result<()> {
    db.with(|c| c.execute("DELETE FROM trash WHERE id = ?1", [id]))?;
    Ok(())
}

/// How a person reaches an entry: through their own scope (restore and delete), or a grant (restore only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Via {
    Own,
    /// Through the grant with this id, on the folder stored at `folder`.
    Grant {
        id: String,
        folder: String,
    },
}

/// The entries `user` sees: those whose original place they may write to now, through their own scope or a live
/// grant that lets them write to a folder holding it. Blocking.
pub fn visible(state: &AppState, user: &crate::users::User) -> Vec<(Entry, Via)> {
    let db = &state.db;
    let Ok(rows) = all(db).inspect_err(|e| tracing::error!(event = "db.error", error = %e)) else {
        return Vec::new();
    };
    let now = crate::now();
    // Folders this person may write to through a grant: (grant id, stored path).
    let granted: Vec<(String, String)> = crate::grants::list_received(db, user.id, now)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|row| {
            crate::grants::live(
                db,
                &state.config.root,
                state.reserved(),
                state.force_fallback,
                row,
                user.id,
                now,
            )
        })
        .filter(|g| g.write && g.is_folder())
        .map(|g| (g.row.id, g.row.path))
        .collect();
    let under = |path: &str, folder: &str| {
        crate::shares::split_scope(path, folder).is_some_and(|r| !r.is_root())
    };
    // An entry whose date has come is gone for everyone, even before the hourly purge removes it.
    rows.into_iter()
        .filter(|e| e.expires_at > now)
        .filter_map(|e| {
            if user.perms.write && crate::shares::split_scope(&e.path, &user.scope).is_some() {
                return Some((e, Via::Own));
            }
            let (id, folder) = granted
                .iter()
                .find(|(_, folder)| under(&e.path, folder))?
                .clone();
            Some((e, Via::Grant { id, folder }))
        })
        .collect()
}
