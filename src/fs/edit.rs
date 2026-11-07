//! New folders and permanent deletes. Deletes act on a link itself, never on what it points to.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags};

use super::list::TEMP_PREFIX;
use super::path::RelPath;
use super::scope::Scope;
use super::{FsError, MAX_DEPTH};

pub fn mkdir(scope: &Scope, path: &RelPath) -> Result<(), FsError> {
    if path.name().is_some_and(|n| n.starts_with(TEMP_PREFIX)) {
        return Err(FsError::InvalidPath);
    }
    let (dir, name) = scope.parent(path)?;
    scope.check_new_name(&dir, &name)?;
    Ok(rustix::fs::mkdirat(
        &dir,
        name.as_os_str(),
        Mode::from_raw_mode(0o755),
    )?)
}

pub fn remove(scope: &Scope, path: &RelPath) -> Result<(), FsError> {
    let (dir, name) = scope.parent(path)?;
    super::mount::refuse_mounts_inside(&dir, name.as_os_str())?;
    remove_at(scope, &dir, &name, 0)
}

/// Removes `name` in `dir`. A folder is emptied level by level, each opened without following links; the data
/// folder, if met on the way, is left alone (and its parent then cannot be removed).
fn remove_at(scope: &Scope, dir: &OwnedFd, name: &OsStr, depth: usize) -> Result<(), FsError> {
    if depth > MAX_DEPTH {
        return Err(FsError::TooDeep);
    }
    let st = rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)?;
    if scope.is_reserved((st.st_dev, st.st_ino)) {
        return Err(FsError::NotFound);
    }
    if FileType::from_raw_mode(st.st_mode) != FileType::Directory {
        return Ok(rustix::fs::unlinkat(dir, name, AtFlags::empty())?);
    }
    let flags = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::RDONLY | OFlags::CLOEXEC;
    let sub = rustix::fs::openat(dir, name, flags, Mode::empty())?;
    let mut names = Vec::new();
    for item in rustix::fs::Dir::read_from(&sub)? {
        let item = item?;
        let raw = item.file_name().to_bytes();
        if raw != b"." && raw != b".." {
            names.push(OsStr::from_bytes(raw).to_os_string());
        }
    }
    let mut first_error = None;
    for child in names {
        if let Err(e) = remove_at(scope, &sub, &child, depth + 1) {
            first_error.get_or_insert(e);
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }
    Ok(rustix::fs::unlinkat(dir, name, AtFlags::REMOVEDIR)?)
}

/// What the client says it last saw of the file.
pub enum Expect {
    /// `If-Match`: the file must still have this ETag.
    Etag(String),
    /// `If-None-Match: *`: the file must not exist yet.
    New,
}

/// Saves `text` over `path` through a temporary file and a rename, so a failed save never leaves half a file.
pub async fn save_text(
    scope: &Scope,
    db: std::sync::Arc<crate::db::Db>,
    path: &RelPath,
    text: Vec<u8>,
    expect: Expect,
    now: i64,
) -> Result<rustix::fs::Stat, FsError> {
    if std::str::from_utf8(&text).is_err() {
        return Err(FsError::NotText);
    }
    let current = match scope.stat(path) {
        Ok(st) => Some(st),
        Err(FsError::NotFound) => None,
        Err(e) => return Err(e),
    };
    match (&expect, &current) {
        (Expect::New, Some(_)) => return Err(FsError::Exists),
        (Expect::Etag(_), None) => return Err(FsError::Changed),
        (Expect::Etag(tag), Some(st)) => {
            match FileType::from_raw_mode(st.st_mode) {
                FileType::Directory => return Err(FsError::IsAFolder),
                // Saving would replace the link with a file; links are edited through their target's own path.
                FileType::Symlink => return Err(FsError::IsALink),
                _ => {}
            }
            if *tag != super::raw::etag(st) {
                return Err(FsError::Changed);
            }
        }
        (Expect::New, None) => {}
    }
    let replace = current.is_some();
    let body = axum::body::Body::from(text);
    let (st, _) = super::upload::upload(scope, db, path, body, None, replace, now).await?;
    Ok(st)
}
