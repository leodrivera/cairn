//! The trash: deleted items wait in a `.cairn-trash` folder at the top of their mount until their date, a restore
//! or a permanent delete. The `trash` table holds one row per entry; nothing else about files is kept.

pub mod folder;

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
