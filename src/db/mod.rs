//! SQLite: one connection behind a mutex, WAL mode, migrations embedded in the binary.

use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

/// Applied in order; `PRAGMA user_version` holds how many have run.
const MIGRATIONS: &[&str] = &[include_str!("migrations/0001_users.sql")];

pub struct Db(Mutex<Connection>);

impl Db {
    pub fn open(path: &Path) -> Result<Db, String> {
        let fail = |e: &dyn std::fmt::Display| {
            format!("Cannot open the database {} ({e}).", path.display())
        };
        // Created owner-only: it holds password hashes and session digests.
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| fail(&e))?;
        let conn = Connection::open(path).map_err(|e| fail(&e))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| fail(&e))?;
        Self::setup(conn).map_err(|e| fail(&e))
    }

    pub fn open_in_memory() -> Db {
        Self::setup(Connection::open_in_memory().expect("in-memory database")).expect("migrations")
    }

    fn setup(mut conn: Connection) -> rusqlite::Result<Db> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        let applied: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let applied = usize::try_from(applied).unwrap_or(0);
        if applied < MIGRATIONS.len() {
            let tx = conn.transaction()?;
            for sql in &MIGRATIONS[applied..] {
                tx.execute_batch(sql)?;
            }
            tx.pragma_update(None, "user_version", MIGRATIONS.len() as i64)?;
            tx.commit()?;
        }
        Ok(Db(Mutex::new(conn)))
    }

    pub fn with<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        let conn = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&conn)
    }

    /// SQLite online backup: a consistent copy while Cairn runs.
    pub fn backup_to(&self, dest: &Path) -> Result<(), String> {
        let conn = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.backup(
            rusqlite::MAIN_DB,
            dest,
            None::<fn(rusqlite::backup::Progress)>,
        )
        .map_err(|e| format!("Cannot write the backup to {} ({e}).", dest.display()))
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn tables(db: &Db) -> Vec<String> {
        db.with(|c| {
            let mut q =
                c.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
            q.query_map([], |r| r.get(0))?.collect()
        })
        .unwrap()
    }

    fn version(db: &Db) -> i64 {
        db.with(|c| c.query_row("PRAGMA user_version", [], |r| r.get(0)))
            .unwrap()
    }

    fn add_user(db: &Db) {
        db.with(|c| {
            c.execute(
                "INSERT INTO users (email, name, created_at) VALUES ('a@example.com', 'A', 0)",
                [],
            )
        })
        .unwrap();
    }

    #[test]
    fn fresh_db_has_tables() {
        let db = Db::open_in_memory();
        assert_eq!(version(&db), 1);
        let t = tables(&db);
        for name in ["sessions", "settings", "users"] {
            assert!(t.contains(&name.to_string()), "{name} missing in {t:?}");
        }
    }

    #[test]
    fn reopen_does_not_rerun() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cairn.db");
        add_user(&Db::open(&path).unwrap());
        let db = Db::open(&path).unwrap();
        assert_eq!(version(&db), 1);
        let n: i64 = db
            .with(|c| c.query_row("SELECT count(*) FROM users", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn foreign_keys_on() {
        let db = Db::open_in_memory();
        let r = db.with(|c| {
            c.execute(
                "INSERT INTO sessions (id, user_id, method, created_at, expires_at) VALUES (x'00', 99, 'password', 0, 1)",
                [],
            )
        });
        assert!(r.is_err());
    }

    #[test]
    fn file_mode_is_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cairn.db");
        Db::open(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn backup_copies_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open_in_memory();
        add_user(&db);
        let dest = dir.path().join("backup.db");
        db.backup_to(&dest).unwrap();
        let copy = Db::open(&dest).unwrap();
        let n: i64 = copy
            .with(|c| c.query_row("SELECT count(*) FROM users", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(n, 1);
    }
}
