//! Rename, move and copy, inside the scope. Links are moved or copied as links, never followed into folders.

use std::os::unix::ffi::OsStrExt;

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags};
use rustix::io::Errno;
use serde::Deserialize;

use super::FsError;
use super::list::TEMP_PREFIX;
use super::path::RelPath;
use super::scope::Scope;

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
    if to.is_root() || to.name().is_some_and(|n| n.starts_with(TEMP_PREFIX)) {
        return Err(FsError::InvalidPath);
    }
    if to.starts_with(from) {
        return Err(FsError::IntoItself);
    }
    if op == Op::Rename && from.parent() != to.parent() {
        return Err(FsError::InvalidPath);
    }
    let (src_dir, src) = scope.parent(from)?;
    rustix::fs::statat(&src_dir, src.as_os_str(), AtFlags::SYMLINK_NOFOLLOW)?;
    let (dst_dir, dst) = scope.parent(to)?;
    if rustix::fs::statat(&dst_dir, dst.as_os_str(), AtFlags::SYMLINK_NOFOLLOW).is_ok() {
        return Err(FsError::Exists);
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
                // Another filesystem (another mount inside the scope): copy, then remove the original.
                Err(Errno::XDEV) => {
                    copy_at(
                        scope,
                        from,
                        &src_dir,
                        src.as_os_str().as_bytes(),
                        &dst_dir,
                        dst.as_os_str().as_bytes(),
                    )?;
                    super::edit::remove(scope, from)
                }
                Err(e) => Err(e.into()),
            }
        }
        Op::Copy => copy_at(
            scope,
            from,
            &src_dir,
            src.as_os_str().as_bytes(),
            &dst_dir,
            dst.as_os_str().as_bytes(),
        ),
    }
}

/// Copies one entry. `rel` is the source's scope-relative path, used to check where links lead.
fn copy_at(
    scope: &Scope,
    rel: &RelPath,
    src_dir: &OwnedFd,
    src: &[u8],
    dst_dir: &OwnedFd,
    dst: &[u8],
) -> Result<(), FsError> {
    let (src_os, dst_os) = (
        std::ffi::OsStr::from_bytes(src),
        std::ffi::OsStr::from_bytes(dst),
    );
    let st = rustix::fs::statat(src_dir, src_os, AtFlags::SYMLINK_NOFOLLOW)?;
    if scope.data_id() == Some((st.st_dev, st.st_ino)) {
        return Ok(());
    }
    match FileType::from_raw_mode(st.st_mode) {
        FileType::Symlink => {
            // Copied as a link only when it stays inside the scope; others are left out.
            if !scope.link_inside(rel) {
                return Ok(());
            }
            let target = rustix::fs::readlinkat(src_dir, src_os, Vec::new())?;
            Ok(rustix::fs::symlinkat(target.as_c_str(), dst_dir, dst_os)?)
        }
        FileType::Directory => {
            let flags = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::RDONLY | OFlags::CLOEXEC;
            let from_dir = rustix::fs::openat(src_dir, src_os, flags, Mode::empty())?;
            rustix::fs::mkdirat(dst_dir, dst_os, Mode::from_raw_mode(st.st_mode & 0o7777))?;
            let to_dir = rustix::fs::openat(dst_dir, dst_os, flags, Mode::empty())?;
            for item in rustix::fs::Dir::read_from(&from_dir)? {
                let item = item?;
                let name = item.file_name().to_bytes().to_vec();
                if name == b"." || name == b".." {
                    continue;
                }
                let child = match std::str::from_utf8(&name)
                    .ok()
                    .and_then(|n| rel.join(n).ok())
                {
                    Some(child) => child,
                    None => continue,
                };
                copy_at(scope, &child, &from_dir, &name, &to_dir, &name)?;
            }
            Ok(())
        }
        FileType::RegularFile => {
            let src_fd = rustix::fs::openat(
                src_dir,
                src_os,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            let create = OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::CLOEXEC;
            let dst_fd = rustix::fs::openat(
                dst_dir,
                dst_os,
                create,
                Mode::from_raw_mode(st.st_mode & 0o7777),
            )?;
            let (mut r, mut w) = (std::fs::File::from(src_fd), std::fs::File::from(dst_fd));
            std::io::copy(&mut r, &mut w)?;
            Ok(())
        }
        // Devices, sockets and pipes are not copied.
        _ => Ok(()),
    }
}
