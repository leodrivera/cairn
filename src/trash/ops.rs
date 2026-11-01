//! Moving into the trash and out of it. A delete writes its row first and then renames, so a crash can leave a row
//! without a file (dropped by `reconcile`), never a file in the trash that Cairn would not know to remove.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags};
use rustix::io::Errno;

use super::Entry;
use super::folder::{Trashes, mount_top};
use crate::db::Db;
use crate::fs::path::RelPath;
use crate::fs::scope::{Reserved, Scope};
use crate::fs::{FsError, MAX_DEPTH};

const DIR: OFlags = OFlags::DIRECTORY
    .union(OFlags::RDONLY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

fn renames_refused() -> &'static Mutex<HashSet<PathBuf>> {
    static SET: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    SET.get_or_init(Default::default)
}

/// Tests only: moving the item at `path` into the trash fails as if it were on another mount (`EXDEV`).
#[doc(hidden)]
pub fn fail_rename_for(path: &Path) {
    let real = std::fs::canonicalize(path).expect("item exists");
    renames_refused().lock().expect("set").insert(real);
}

fn refused(dir: &OwnedFd, name: &OsStr) -> bool {
    let set = renames_refused().lock().expect("set");
    !set.is_empty()
        && std::fs::read_link(format!(
            "/proc/self/fd/{}",
            rustix::fd::AsRawFd::as_raw_fd(dir)
        ))
        .is_ok_and(|d| set.contains(&d.join(name)))
}

/// Tests only: the folder `dir` itself is in the refused set, so a restore into it fails with `EXDEV`.
fn refused_into(dir: &OwnedFd) -> bool {
    let set = renames_refused().lock().expect("set");
    !set.is_empty()
        && std::fs::read_link(format!(
            "/proc/self/fd/{}",
            rustix::fd::AsRawFd::as_raw_fd(dir)
        ))
        .is_ok_and(|d| set.contains(&d))
}

fn db_err(e: rusqlite::Error) -> FsError {
    FsError::Io(std::io::Error::other(e))
}

/// Bytes of the files under `name`; what cannot be read, and links, count nothing.
fn size_of(dir: &OwnedFd, name: &OsStr, depth: usize) -> u64 {
    let Ok(st) = rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) else {
        return 0;
    };
    match FileType::from_raw_mode(st.st_mode) {
        FileType::RegularFile => st.st_size as u64,
        FileType::Directory if depth < MAX_DEPTH => {
            let Ok(sub) = rustix::fs::openat(dir, name, DIR, Mode::empty()) else {
                return 0;
            };
            let Ok(items) = rustix::fs::Dir::read_from(&sub) else {
                return 0;
            };
            let names: Vec<Vec<u8>> = items
                .filter_map(Result::ok)
                .map(|i| i.file_name().to_bytes().to_vec())
                .filter(|n| n != b"." && n != b"..")
                .collect();
            names
                .iter()
                .map(|n| size_of(&sub, OsStr::from_bytes(n), depth + 1))
                .sum()
        }
        _ => 0,
    }
}

fn open_root(trashes: &Trashes) -> Result<OwnedFd, FsError> {
    Ok(rustix::fs::open(trashes.root(), DIR, Mode::empty())?)
}

/// Moves `rel`, beneath `scope` whose own folder is `base` (stored form), into its mount's trash. Returns the
/// entry id.
#[allow(clippy::too_many_arguments)]
pub fn to_trash(
    trashes: &Trashes,
    db: &Db,
    scope: &Scope,
    base: &str,
    rel: &RelPath,
    user: i64,
    now: i64,
    days: u32,
) -> Result<String, FsError> {
    if days == 0 {
        return Err(FsError::TrashOff);
    }
    let _moving = trashes.moving();
    let (dir, name) = scope.parent(rel)?;
    crate::fs::mount::refuse_mounts_inside(&dir, name.as_os_str())?;
    let st = rustix::fs::statat(&dir, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW)?;
    let top = mount_top(&open_root(trashes)?, &dir)?;
    let (trash, trash_dir) = trashes.for_dir(db, &top)?;
    let size = size_of(&dir, name.as_os_str(), 0);
    let kind = if FileType::from_raw_mode(st.st_mode) == FileType::Directory {
        "folder"
    } else {
        "file"
    };
    let entry = Entry {
        id: crate::shares::new_id(),
        trash_dir,
        path: crate::shares::root_path(base, rel),
        name: name.as_bytes().to_vec(),
        kind: kind.into(),
        size: size as i64,
        deleted_by: Some(user),
        deleted_at: now,
        expires_at: now + i64::from(days) * 86_400,
    };
    super::insert(db, &entry).map_err(db_err)?;
    let undo = |e: FsError| {
        let _ = rustix::fs::unlinkat(&trash, entry.id.as_str(), AtFlags::REMOVEDIR);
        let _ = super::delete_row(db, &entry.id);
        e
    };
    rustix::fs::mkdirat(&trash, entry.id.as_str(), Mode::from_raw_mode(0o700))
        .map_err(|e| undo(e.into()))?;
    let holder = rustix::fs::openat(&trash, entry.id.as_str(), DIR, Mode::empty())
        .map_err(|e| undo(e.into()))?;
    let moved = if refused(&dir, &name) {
        Err(Errno::XDEV)
    } else {
        rustix::fs::renameat_with(
            &dir,
            name.as_os_str(),
            &holder,
            name.as_os_str(),
            RenameFlags::NOREPLACE,
        )
    };
    match moved {
        Ok(()) => {
            tracing::info!(event = "trash.move", user, kind, size);
            Ok(entry.id)
        }
        Err(Errno::XDEV) => Err(undo(FsError::OtherDisk(
            name.to_string_lossy().into_owned(),
        ))),
        Err(e) => Err(undo(e.into())),
    }
}

/// The folder holding `entry`'s trash folder, as a scope that may reach inside it.
fn trash_scope(trashes: &Trashes, entry: &Entry) -> Result<Scope, FsError> {
    if !trashes.ours().contains(&entry.trash_dir) {
        return Err(FsError::NotFound);
    }
    Scope::new(trashes.root(), &entry.trash_dir, Reserved::default())
}

/// `name (restored).ext` for files, `name (restored)` for folders; `n` above 1 adds the number.
fn restored_name(name: &[u8], folder: bool, n: usize) -> Vec<u8> {
    let tag = if n == 1 {
        " (restored)".to_string()
    } else {
        format!(" (restored {n})")
    };
    let dot = (!folder)
        .then(|| name.iter().rposition(|&b| b == b'.'))
        .flatten()
        .filter(|&i| i > 0);
    match dot {
        Some(i) => [&name[..i], tag.as_bytes(), &name[i..]].concat(),
        None => [name, tag.as_bytes()].concat(),
    }
}

/// Puts `entry` back beneath `scope` (whose own folder is `base`), recreating missing folders. Returns where it
/// landed, relative to the scope.
pub fn restore(
    trashes: &Trashes,
    db: &Db,
    scope: &Scope,
    base: &str,
    entry: &Entry,
) -> Result<RelPath, FsError> {
    let _moving = trashes.moving();
    let rel = crate::shares::split_scope(&entry.path, base).ok_or(FsError::NotFound)?;
    let parent = rel.parent().ok_or(FsError::ScopeRoot)?;
    let holder_scope = trash_scope(trashes, entry)?;
    let holder = holder_scope.open_dir(&RelPath::parse(&entry.id)?)?;
    let name = OsStr::from_bytes(&entry.name);
    let st = rustix::fs::statat(&holder, name, AtFlags::SYMLINK_NOFOLLOW)?;
    let folder = FileType::from_raw_mode(st.st_mode) == FileType::Directory;
    for n in 1..=parent.len() {
        match crate::fs::edit::mkdir(scope, &parent.prefix(n)) {
            Ok(()) | Err(FsError::Exists) => {}
            Err(e) => return Err(e),
        }
    }
    let dst = scope.open_dir(&parent)?;
    for n in 0..1000 {
        let candidate = if n == 0 {
            entry.name.clone()
        } else {
            restored_name(&entry.name, folder, n)
        };
        let cand = OsStr::from_bytes(&candidate);
        scope.check_new_name(&dst, cand)?;
        let moved = if refused_into(&dst) {
            Err(Errno::XDEV)
        } else {
            rustix::fs::renameat_with(&holder, name, &dst, cand, RenameFlags::NOREPLACE)
        };
        match moved {
            Ok(()) => {
                let _ = rustix::fs::unlinkat(
                    &holder_scope.open_dir(&RelPath::parse("")?)?,
                    entry.id.as_str(),
                    AtFlags::REMOVEDIR,
                );
                super::delete_row(db, &entry.id).map_err(db_err)?;
                tracing::info!(event = "trash.restore", renamed = n > 0);
                return parent.join_bytes(&candidate);
            }
            Err(Errno::EXIST) => continue,
            Err(Errno::XDEV) => {
                return Err(FsError::RestoreOtherDisk(
                    name.to_string_lossy().into_owned(),
                ));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(FsError::Exists)
}

/// Deletes `entry` for good: its folder in the trash, then its row.
pub fn remove(trashes: &Trashes, db: &Db, entry: &Entry) -> Result<(), FsError> {
    let _moving = trashes.moving();
    remove_held(trashes, db, entry)
}

fn remove_held(trashes: &Trashes, db: &Db, entry: &Entry) -> Result<(), FsError> {
    match trash_scope(trashes, entry)
        .and_then(|s| crate::fs::edit::remove(&s, &RelPath::parse(&entry.id)?))
    {
        Ok(()) | Err(FsError::NotFound) => {}
        Err(e) => return Err(e),
    }
    super::delete_row(db, &entry.id).map_err(db_err)?;
    tracing::info!(event = "trash.delete", size = entry.size);
    Ok(())
}

/// Removes every entry whose date has come. Returns how many went.
pub fn purge(trashes: &Trashes, db: &Db, now: i64) -> usize {
    let _upkeep = trashes.upkeep();
    trashes.refresh(db);
    let Ok(rows) = super::all(db) else {
        return 0;
    };
    let count = rows
        .iter()
        .filter(|e| e.expires_at <= now)
        .filter(|e| {
            remove_held(trashes, db, e)
                .inspect_err(|err| tracing::warn!(event = "trash.purge_failed", error = ?err))
                .is_ok()
        })
        .count();
    if count > 0 {
        tracing::info!(event = "trash.purged", count);
    }
    count
}

/// Drops rows whose item is gone from a trash folder of ours, and counts folders there that no row names (left
/// alone). A trash folder that is not there right now (an unplugged disk) keeps its rows.
pub fn reconcile(trashes: &Trashes, db: &Db) -> (usize, usize) {
    let _upkeep = trashes.upkeep();
    trashes.refresh(db);
    let Ok(rows) = super::all(db) else {
        return (0, 0);
    };
    let mut dropped = 0;
    for e in &rows {
        let Ok(scope) = trash_scope(trashes, e) else {
            continue;
        };
        let item = RelPath::parse(&e.id).and_then(|p| p.join_bytes(&e.name));
        if matches!(item.and_then(|p| scope.stat(&p)), Err(FsError::NotFound)) {
            // An empty holder folder left by the crash goes too; one with anything in it stays.
            if let Ok(here) = scope.open_dir(&RelPath::parse("").expect("root")) {
                let _ = rustix::fs::unlinkat(&here, e.id.as_str(), AtFlags::REMOVEDIR);
            }
            if super::delete_row(db, &e.id).is_ok() {
                dropped += 1;
            }
        }
    }
    let known: HashSet<&str> = rows.iter().map(|e| e.id.as_str()).collect();
    let mut unknown = 0;
    for folder in trashes.ours() {
        let Ok(scope) = Scope::new(trashes.root(), &folder, Reserved::default()) else {
            continue;
        };
        let Ok(list) = crate::fs::list::list(&scope, &RelPath::parse("").expect("root")) else {
            continue;
        };
        unknown += list
            .iter()
            .filter(|e| e.name != super::folder::MARKER && !known.contains(e.name.as_str()))
            .count();
    }
    if dropped > 0 || unknown > 0 {
        tracing::info!(event = "trash.reconciled", dropped, unknown);
    }
    (dropped, unknown)
}

#[cfg(test)]
mod tests {
    use super::restored_name;

    #[test]
    fn restored_names() {
        assert_eq!(
            restored_name(b"report.txt", false, 1),
            b"report (restored).txt"
        );
        assert_eq!(
            restored_name(b"report.txt", false, 2),
            b"report (restored 2).txt"
        );
        assert_eq!(
            restored_name(b"site-backup.tar.gz", false, 1),
            b"site-backup.tar (restored).gz"
        );
        assert_eq!(restored_name(b".env", false, 1), b".env (restored)");
        assert_eq!(
            restored_name(b"Archive.v2", true, 1),
            b"Archive.v2 (restored)"
        );
    }
}
