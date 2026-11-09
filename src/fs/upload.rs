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
    abs: Vec<u8>,
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

/// The absolute path of `name` in `dir`, as recorded in `uploads` for the startup cleanup: its exact bytes, since a
/// folder name need not be UTF-8.
fn stored_path(dir: &OwnedFd, name: &str) -> std::io::Result<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    let folder = std::fs::read_link(format!("/proc/self/fd/{}", dir.as_raw_fd()))?;
    Ok(folder.join(name).as_os_str().as_bytes().to_vec())
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
    if path
        .name_bytes()
        .is_none_or(|n| n.starts_with(TEMP_PREFIX.as_bytes()))
    {
        return Err(FsError::InvalidPath);
    }
    let (dir, name) = scope.parent(path)?;
    scope.check_new_name(&dir, &name)?;
    let existing = rustix::fs::statat(&dir, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW).ok();
    if let Some(st) = &existing {
        if FileType::from_raw_mode(st.st_mode) == FileType::Directory {
            return Err(FsError::IsAFolder);
        }
        if !replace {
            return Err(FsError::Exists);
        }
        // Replacing would swap the link for a file, with the link's own 0777 mode; as in `save_text`, links are
        // written through their target's own path.
        if FileType::from_raw_mode(st.st_mode) == FileType::Symlink {
            return Err(FsError::IsALink);
        }
    }

    let mut random = [0u8; 8];
    getrandom::fill(&mut random).expect("system random source");
    let temp_name = format!("{TEMP_PREFIX}{:016x}", u64::from_le_bytes(random));
    let flags = OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::CLOEXEC;
    let fd = rustix::fs::openat(&dir, temp_name.as_str(), flags, Mode::from_raw_mode(0o644))?;
    let abs = stored_path(&dir, &temp_name)?;
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
    // Writes run in the background: only this reports a failed last write, and `into_std` would drop that error.
    file.flush().await?;
    let file = file.into_std().await;
    // The final fsync waits for the disk, which on a slow one takes seconds: on the blocking pool, so the runtime keeps
    // serving everyone else meanwhile.
    let old = existing;
    tokio::task::spawn_blocking(move || -> Result<(), FsError> {
        if let Some(old) = &old {
            keep_owner_and_mode(&file, old)?;
        }
        file.sync_all()?;
        Ok(())
    })
    .await
    .map_err(|e| FsError::Io(std::io::Error::other(e)))??;

    let how = if replace {
        RenameFlags::empty()
    } else {
        RenameFlags::NOREPLACE
    };
    rustix::fs::renameat_with(
        &temp.dir,
        temp.name.as_str(),
        &temp.dir,
        name.as_os_str(),
        how,
    )?;
    let mut temp = temp;
    temp.armed = false;
    let st = rustix::fs::statat(&temp.dir, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW)?;
    Ok((st, existing.is_some()))
}

/// The replacement takes the original's permissions, so a script keeps its execute bit. It also takes the
/// original's owner when the process may set it: as root, or when Cairn already runs as that owner (the `user:` of
/// the Compose file). Otherwise the new file belongs to the user running Cairn.
fn keep_owner_and_mode(file: &std::fs::File, old: &rustix::fs::Stat) -> Result<(), FsError> {
    // Without setuid, setgid and sticky: replacing a privileged program's content must not carry its privilege.
    rustix::fs::fchmod(file, Mode::from_raw_mode(old.st_mode & 0o777))?;
    let owner = (
        rustix::fs::Uid::from_raw(old.st_uid),
        rustix::fs::Gid::from_raw(old.st_gid),
    );
    match rustix::fs::fchown(file, Some(owner.0), Some(owner.1)) {
        Ok(()) | Err(rustix::io::Errno::PERM) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// At startup no upload is running, so every recorded temporary file is a leftover from a crash.
pub fn cleanup_leftovers(db: &Db) {
    use std::os::unix::ffi::OsStrExt;
    let paths: Vec<Vec<u8>> = db
        .with(|c| {
            let mut q = c.prepare("SELECT CAST(path AS BLOB) FROM uploads")?;
            q.query_map([], |r| r.get(0))?.collect()
        })
        .unwrap_or_default();
    for p in &paths {
        let p = std::path::Path::new(std::ffi::OsStr::from_bytes(p));
        if p.file_name()
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
    /// A crash leaves the temporary file and its row; startup removes it, whatever bytes its folder's name has.
    #[test]
    fn leftover_in_a_folder_whose_name_is_not_utf8_is_removed() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join(std::ffi::OsStr::from_bytes(b"up\xff"));
        std::fs::create_dir(&folder).unwrap();
        let name = format!("{TEMP_PREFIX}0000000000000001");
        std::fs::write(folder.join(&name), "partial").unwrap();
        let dir = rustix::fs::open(
            &folder,
            OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        let db = Db::open_in_memory();
        let path = stored_path(&dir, &name).unwrap();
        db.with(|c| {
            c.execute(
                "INSERT INTO uploads (path, started_at) VALUES (?1, 0)",
                rusqlite::params![path],
            )
        })
        .unwrap();
        cleanup_leftovers(&db);
        assert!(!folder.join(&name).exists());
    }

    use std::sync::Arc;

    use super::*;

    fn setup() -> (tempfile::TempDir, Scope, Arc<Db>) {
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::new(dir.path(), "/", Default::default()).unwrap();
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

    /// Runs only inside `failed_last_write_keeps_the_original`, under its file size limit.
    #[tokio::test]
    #[ignore = "started by failed_last_write_keeps_the_original"]
    async fn replace_past_the_file_size_limit() {
        let (dir, scope, db) = setup();
        let target = dir.path().join("note.txt");
        std::fs::write(&target, "old").unwrap();
        let body = axum::body::Body::from(vec![b'x'; 64 * 1024]);
        let path = RelPath::parse("note.txt").unwrap();
        let result = upload(&scope, db.clone(), &path, body, None, true, 0).await;
        assert!(result.is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert_eq!(rows(&db), 0);
    }

    /// The size limit makes the last write fail for real, on any filesystem and with no privileges. The ignored
    /// SIGXFSZ survives `exec`, so that write answers EFBIG instead of ending the process.
    #[test]
    fn failed_last_write_keeps_the_original() {
        let test = std::env::current_exe().unwrap();
        let out = std::process::Command::new("sh")
            .args(["-c", "trap '' XFSZ; ulimit -f 8; exec \"$0\" \"$@\""])
            .arg(test)
            .args([
                "--exact",
                "--ignored",
                "--nocapture",
                "fs::upload::tests::replace_past_the_file_size_limit",
            ])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let errors = String::from_utf8_lossy(&out.stderr);
        assert!(text.contains("1 passed"), "{text}{errors}");
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
