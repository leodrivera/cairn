//! Rename, move and copy, inside the scope. Links are moved or copied as links, never followed into folders.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags};
use rustix::io::Errno;
use serde::Deserialize;

use super::list::TEMP_PREFIX;
use super::path::RelPath;
use super::scope::Scope;
use super::{FsError, MAX_DEPTH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Rename,
    Move,
    Copy,
}

pub fn apply(scope: &Scope, op: Op, from: &RelPath, to: &RelPath) -> Result<(), FsError> {
    if from.is_root() {
        return Err(FsError::ScopeRoot);
    }
    if to.is_root()
        || to
            .name_bytes()
            .is_some_and(|n| n.starts_with(TEMP_PREFIX.as_bytes()))
    {
        return Err(FsError::InvalidPath);
    }
    if to.starts_with(from) {
        return Err(FsError::IntoItself);
    }
    if op == Op::Rename && from.parent() != to.parent() {
        return Err(FsError::InvalidPath);
    }
    let (src_dir, src) = scope.parent(from)?;
    let src_st = rustix::fs::statat(&src_dir, src.as_os_str(), AtFlags::SYMLINK_NOFOLLOW)?;
    let (dst_dir, dst) = scope.parent(to)?;
    if rustix::fs::statat(&dst_dir, dst.as_os_str(), AtFlags::SYMLINK_NOFOLLOW).is_ok() {
        return Err(FsError::Exists);
    }
    // The lexical check above misses a destination reached through a link (`L -> a`, then `a` to `L/sub`):
    // walk up from the destination folder and refuse if the source folder is on the way.
    if FileType::from_raw_mode(src_st.st_mode) == FileType::Directory
        && inside(scope, &dst_dir, id(&src_st))?
    {
        return Err(FsError::IntoItself);
    }
    match op {
        Op::Rename | Op::Move => {
            match rustix::fs::renameat_with(
                &src_dir,
                src.as_os_str(),
                &dst_dir,
                dst.as_os_str(),
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => Ok(()),
                Err(Errno::INVAL) => Err(FsError::IntoItself),
                // Another filesystem (another mount inside the scope): copy, then remove the original, but only
                // when everything was copied.
                Err(Errno::XDEV) => {
                    let skipped = copy_tree(scope, from, &src_dir, &src, &dst_dir, &dst)?;
                    if skipped > 0 {
                        return Err(FsError::Skipped {
                            count: skipped,
                            kept: true,
                        });
                    }
                    super::edit::remove(scope, from)
                }
                Err(e) => Err(e.into()),
            }
        }
        Op::Copy => match copy_tree(scope, from, &src_dir, &src, &dst_dir, &dst)? {
            0 => Ok(()),
            count => Err(FsError::Skipped { count, kept: false }),
        },
    }
}

fn id(st: &rustix::fs::Stat) -> (u64, u64) {
    (st.st_dev, st.st_ino)
}

/// Whether the folder `target` is `dir` or one of its parents, up to the scope root.
fn inside(scope: &Scope, dir: &OwnedFd, target: (u64, u64)) -> Result<bool, FsError> {
    let root = scope.root_id()?;
    let mut cur = rustix::fs::openat(
        dir,
        ".",
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    for _ in 0..=MAX_DEPTH * 16 {
        let here = id(&rustix::fs::fstat(&cur)?);
        if here == target {
            return Ok(true);
        }
        if here == root {
            return Ok(false);
        }
        let up = rustix::fs::openat(
            &cur,
            "..",
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        if id(&rustix::fs::fstat(&up)?) == here {
            return Ok(false); // the filesystem root
        }
        cur = up;
    }
    Ok(false)
}

struct Copy<'a> {
    scope: &'a Scope,
    /// The new top folder, never copied into itself even if the source walk meets it.
    created: Option<(u64, u64)>,
    skipped: usize,
}

/// Copies `src` to `dst` and returns how many entries were left out.
fn copy_tree(
    scope: &Scope,
    rel: &RelPath,
    src_dir: &OwnedFd,
    src: &OsStr,
    dst_dir: &OwnedFd,
    dst: &OsStr,
) -> Result<usize, FsError> {
    let mut ctx = Copy {
        scope,
        created: None,
        skipped: 0,
    };
    copy_at(&mut ctx, rel, src_dir, src, dst_dir, dst, 0)?;
    Ok(ctx.skipped)
}

/// Copies one entry. `rel` is the source's scope-relative path, used to check where links lead.
fn copy_at(
    ctx: &mut Copy,
    rel: &RelPath,
    src_dir: &OwnedFd,
    src: &OsStr,
    dst_dir: &OwnedFd,
    dst: &OsStr,
    depth: usize,
) -> Result<(), FsError> {
    if depth > MAX_DEPTH {
        return Err(FsError::TooDeep);
    }
    let st = rustix::fs::statat(src_dir, src, AtFlags::SYMLINK_NOFOLLOW)?;
    if ctx.scope.data_id() == Some(id(&st)) || ctx.created == Some(id(&st)) {
        return Ok(());
    }
    match FileType::from_raw_mode(st.st_mode) {
        FileType::Symlink => {
            // Copied as a link only when it stays inside the scope; others are left out and counted.
            if !ctx.scope.link_inside(rel) {
                ctx.skipped += 1;
                return Ok(());
            }
            let target = rustix::fs::readlinkat(src_dir, src, Vec::new())?;
            Ok(rustix::fs::symlinkat(target.as_c_str(), dst_dir, dst)?)
        }
        FileType::Directory => {
            let flags = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::RDONLY | OFlags::CLOEXEC;
            let from_dir = rustix::fs::openat(src_dir, src, flags, Mode::empty())?;
            rustix::fs::mkdirat(dst_dir, dst, Mode::from_raw_mode(st.st_mode & 0o7777))?;
            let to_dir = rustix::fs::openat(dst_dir, dst, flags, Mode::empty())?;
            if ctx.created.is_none() {
                ctx.created = Some(id(&rustix::fs::fstat(&to_dir)?));
            }
            let mut names = Vec::new();
            for item in rustix::fs::Dir::read_from(&from_dir)? {
                let raw = item?.file_name().to_bytes().to_vec();
                if raw != b"." && raw != b".." {
                    names.push(raw);
                }
            }
            for name in names {
                let Ok(child) = rel.join_bytes(&name) else {
                    ctx.skipped += 1;
                    continue;
                };
                let name = OsStr::from_bytes(&name);
                copy_at(ctx, &child, &from_dir, name, &to_dir, name, depth + 1)?;
            }
            Ok(())
        }
        FileType::RegularFile => {
            let src_fd = rustix::fs::openat(
                src_dir,
                src,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            let create = OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::CLOEXEC;
            let dst_fd = rustix::fs::openat(
                dst_dir,
                dst,
                create,
                Mode::from_raw_mode(st.st_mode & 0o7777),
            )?;
            let (mut r, mut w) = (std::fs::File::from(src_fd), std::fs::File::from(dst_fd));
            std::io::copy(&mut r, &mut w)?;
            Ok(())
        }
        // Devices, sockets and pipes are not copied.
        _ => {
            ctx.skipped += 1;
            Ok(())
        }
    }
}
