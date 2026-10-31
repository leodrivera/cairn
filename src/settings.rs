//! Settings an admin changes in the UI: the default theme and image thumbnails. Security and networking are never
//! here; they come from the environment only.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::users::UserError;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub theme: Option<String>,
    pub thumbnails: bool,
    /// Days new trash entries are kept; left out of a save, it stays as it was.
    #[serde(default, rename = "trashDays", skip_serializing_if = "Option::is_none")]
    pub trash_days: Option<u32>,
}

fn get(c: &rusqlite::Connection, key: &str) -> rusqlite::Result<Option<String>> {
    c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
}

pub fn load(db: &Db) -> rusqlite::Result<Settings> {
    db.with(|c| {
        Ok(Settings {
            theme: get(c, "default_theme")?,
            thumbnails: get(c, "thumbnails")?.as_deref() == Some("1"),
            trash_days: get(c, "trash_days")?.and_then(|v| v.parse().ok()),
        })
    })
}

pub fn save(db: &Db, s: &Settings) -> Result<(), UserError> {
    if let Some(t) = &s.theme
        && t != "dark"
        && t != "light"
    {
        return Err(UserError::Invalid("Theme must be dark or light."));
    }
    if s.trash_days.is_some_and(|d| d > crate::trash::MAX_DAYS) {
        return Err(UserError::Invalid("Keep trash between 0 and 3650 days."));
    }
    db.with(|c| {
        let tx = c.unchecked_transaction()?;
        match &s.theme {
            Some(t) => tx.execute(
                "INSERT INTO settings (key, value) VALUES ('default_theme', ?1)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                [t],
            )?,
            None => tx.execute("DELETE FROM settings WHERE key = 'default_theme'", [])?,
        };
        tx.execute(
            "INSERT INTO settings (key, value) VALUES ('thumbnails', ?1)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![if s.thumbnails { "1" } else { "0" }],
        )?;
        if let Some(d) = s.trash_days {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES ('trash_days', ?1)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![d.to_string()],
            )?;
        }
        tx.commit()
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default() {
        let db = Db::open_in_memory();
        assert_eq!(load(&db).unwrap(), Settings::default());
    }

    #[test]
    fn settings_round_trip() {
        let db = Db::open_in_memory();
        let s = Settings {
            theme: Some("light".into()),
            thumbnails: true,
            trash_days: None,
        };
        save(&db, &s).unwrap();
        assert_eq!(load(&db).unwrap(), s);
        save(&db, &Settings::default()).unwrap();
        assert_eq!(load(&db).unwrap(), Settings::default());
    }

    #[test]
    fn settings_bad_theme() {
        let db = Db::open_in_memory();
        let s = Settings {
            theme: Some("blue".into()),
            thumbnails: false,
            trash_days: None,
        };
        assert!(matches!(save(&db, &s), Err(UserError::Invalid(_))));
        assert_eq!(load(&db).unwrap(), Settings::default());
    }
}
