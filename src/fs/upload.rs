//! Uploads: streamed into a temporary file next to the target, then renamed into place.

use std::sync::Arc;

use futures_util::StreamExt;
use rustix::fd::{AsRawFd, OwnedFd};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags};
use tokio::io::AsyncWriteExt;

use super::FsError;
use super::list::TEMP_PREFIX;
use super::path::RelPath;
use super::scope::Scope;
use crate::db::Db;

fn db_err(e: rusqlite::Error) -> FsError {
    FsError::Io(std::io::Error::other(e))
}

/// The temporary file and its row in `uploads`. Dropping it while armed removes both, so an upload that fails,
/// or whose request is dropped mid-stream, leaves nothing behind.
struct Temp {
    dir: OwnedFd,
    name: String,
    abs: String,
    db: Arc<Db>,
    armed: bool,
}

impl Drop for Temp {
    fn drop(&mut self) {
        if self.armed {
            let _ = rustix::fs::unlinkat(&self.dir, self.name.as_str(), AtFlags::empty());
        }
        let _ = self
            .db
            .with(|c| c.execute("DELETE FROM uploads WHERE path = ?1", [&self.abs]));
    }
}

/// Streams `body` into `path`. Returns the new file's stat and whether it replaced one.
pub async fn upload(
    scope: &Scope,
    db: Arc<Db>,
    path: &RelPath,
    body: axum::body::Body,
    max: Option<u64>,
    replace: bool,
    now: i64,
) -> Result<(rustix::fs::Stat, bool), FsError> {
    if path.name().is_none_or(|n| n.starts_with(TEMP_PREFIX)) {
        return Err(FsError::InvalidPath);
    }
    let (dir, name) = scope.parent(path)?;
    let existing = rustix::fs::statat(&dir, name.as_str(), AtFlags::SYMLINK_NOFOLLOW).ok();
    if let Some(st) = &existing {
        if FileType::from_raw_mode(st.st_mode) == FileType::Directory {
            return Err(FsError::IsAFolder);
        }
        if !replace {
            return Err(FsError::Exists);
        }
    }

    let mut random = [0u8; 8];
    getrandom::fill(&mut random).expect("system random source");
    let temp_name = format!("{TEMP_PREFIX}{:016x}", u64::from_le_bytes(random));
    let flags = OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::CLOEXEC;
    let fd = rustix::fs::openat(&dir, temp_name.as_str(), flags, Mode::from_raw_mode(0o644))?;
    let folder = std::fs::read_link(format!("/proc/self/fd/{}", dir.as_raw_fd()))?;
    let abs = folder.join(&temp_name).to_string_lossy().into_owned();
    let temp = Temp {
        dir,
        name: temp_name,
        abs,
        db,
        armed: true,
    };
    temp.db
        .with(|c| {
            c.execute(
                "INSERT INTO uploads (path, started_at) VALUES (?1, ?2)",
                rusqlite::params![temp.abs, now],
            )
        })
        .map_err(db_err)?;

    let mut file = tokio::fs::File::from_std(std::fs::File::from(fd));
    let mut written: u64 = 0;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| FsError::Io(std::io::Error::other(e)))?;
        written += chunk.len() as u64;
        if let Some(limit) = max
            && written > limit
        {
            return Err(FsError::TooLarge(limit));
        }
        file.write_all(&chunk).await?;
    }
    file.sync_all().await?;
    drop(file);

    let how = if replace {
        RenameFlags::empty()
    } else {
        RenameFlags::NOREPLACE
    };
    rustix::fs::renameat_with(&temp.dir, temp.name.as_str(), &temp.dir, name.as_str(), how)?;
    let mut temp = temp;
    temp.armed = false;
    let st = rustix::fs::statat(&temp.dir, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)?;
    Ok((st, existing.is_some()))
}

/// At startup no upload is running, so every recorded temporary file is a leftover from a crash.
pub fn cleanup_leftovers(db: &Db) {
    let paths: Vec<String> = db
        .with(|c| {
            let mut q = c.prepare("SELECT path FROM uploads")?;
            q.query_map([], |r| r.get(0))?.collect()
        })
        .unwrap_or_default();
    for p in &paths {
        if std::path::Path::new(p)
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(TEMP_PREFIX))
        {
            let _ = std::fs::remove_file(p);
        }
    }
    let _ = db.with(|c| c.execute("DELETE FROM uploads", []));
    if !paths.is_empty() {
        tracing::info!(
            event = "startup.upload_leftovers_removed",
            count = paths.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn setup() -> (tempfile::TempDir, Scope, Arc<Db>) {
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::new(dir.path(), "/", None).unwrap();
        (dir, scope, Arc::new(Db::open_in_memory()))
    }

    fn rows(db: &Db) -> i64 {
        db.with(|c| c.query_row("SELECT count(*) FROM uploads", [], |r| r.get(0)))
            .unwrap()
    }

    #[tokio::test]
    async fn cut_off_upload_leaves_nothing() {
        let (dir, scope, db) = setup();
        let chunks: Vec<Result<&'static [u8], std::io::Error>> = vec![
            Ok(b"aaa"),
            Ok(b"bbb"),
            Ok(b"ccc"),
            Err(std::io::Error::other("connection reset")),
        ];
        let body = axum::body::Body::from_stream(futures_util::stream::iter(chunks));
        let path = RelPath::parse("cut.bin").unwrap();
        assert!(
            upload(&scope, db.clone(), &path, body, None, false, 0)
                .await
                .is_err()
        );
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(left.is_empty(), "{left:?}");
        assert_eq!(rows(&db), 0);
    }

    #[tokio::test]
    async fn dropped_upload_leaves_nothing() {
        let (dir, scope, db) = setup();
        // A body that never ends: the client went quiet, then the server dropped the request.
        let body = axum::body::Body::from_stream(futures_util::stream::pending::<
            Result<&'static [u8], std::io::Error>,
        >());
        let path = RelPath::parse("hang.bin").unwrap();
        let fut = upload(&scope, db.clone(), &path, body, None, false, 0);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), fut)
                .await
                .is_err()
        );
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(left.is_empty(), "{left:?}");
        assert_eq!(rows(&db), 0);
    }

    #[test]
    fn startup_removes_leftovers() {
        let (dir, _scope, db) = setup();
        let stale = dir.path().join(".cairn-upload-0000000000000000");
        std::fs::write(&stale, "partial").unwrap();
        db.with(|c| {
            c.execute(
                "INSERT INTO uploads (path, started_at) VALUES (?1, 0)",
                [stale.to_str().unwrap()],
            )
        })
        .unwrap();
        cleanup_leftovers(&db);
        assert!(!stale.exists());
        assert_eq!(rows(&db), 0);
    }
}
