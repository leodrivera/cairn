//! New folders and permanent deletes. Deletes act on a link itself, never on what it points to.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags};

use super::FsError;
use super::list::TEMP_PREFIX;
use super::path::RelPath;
use super::scope::Scope;

pub fn mkdir(scope: &Scope, path: &RelPath) -> Result<(), FsError> {
    if path.name().is_some_and(|n| n.starts_with(TEMP_PREFIX)) {
        return Err(FsError::InvalidPath);
    }
    let (dir, name) = scope.parent(path)?;
    Ok(rustix::fs::mkdirat(
        &dir,
        name.as_os_str(),
        Mode::from_raw_mode(0o755),
    )?)
}

pub fn remove(scope: &Scope, path: &RelPath) -> Result<(), FsError> {
    let (dir, name) = scope.parent(path)?;
    remove_at(scope, &dir, &name)
}

/// Removes `name` in `dir`. A folder is emptied level by level, each opened without following links; the data
/// folder, if met on the way, is left alone (and its parent then cannot be removed).
fn remove_at(scope: &Scope, dir: &OwnedFd, name: &OsStr) -> Result<(), FsError> {
    let st = rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)?;
    if scope.data_id() == Some((st.st_dev, st.st_ino)) {
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
        if let Err(e) = remove_at(scope, &sub, &child) {
            first_error.get_or_insert(e);
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }
    Ok(rustix::fs::unlinkat(dir, name, AtFlags::REMOVEDIR)?)
}
