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
        };
        assert!(matches!(save(&db, &s), Err(UserError::Invalid(_))));
        assert_eq!(load(&db).unwrap(), Settings::default());
    }
}
