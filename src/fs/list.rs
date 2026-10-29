//! Folder listings: folders first, then natural name order, so the browser can window them without sorting.

use std::cmp::Ordering;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use rustix::fs::{AtFlags, FileType, OFlags};
use serde::Serialize;

use super::FsError;
use super::mime;
use super::path::RelPath;
use super::scope::Scope;

/// Names Cairn uses for uploads in progress; never listed, never a target.
pub const TEMP_PREFIX: &str = ".cairn-upload-";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Folder,
    File,
    Link,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    pub modified: i64,
    pub mime: &'static str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub outside: bool,
}

/// Case-insensitive, with runs of digits compared as numbers: file2 before file10.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut run = String::new();
                    while let Some(ch) = it.peek().copied().filter(char::is_ascii_digit) {
                        run.push(ch);
                        it.next();
                    }
                    run
                };
                let (r, s) = (take(&mut x), take(&mut y));
                let (rt, st) = (r.trim_start_matches('0'), s.trim_start_matches('0'));
                let ord = rt
                    .len()
                    .cmp(&st.len())
                    .then_with(|| rt.cmp(st))
                    .then_with(|| r.len().cmp(&s.len()));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(c), Some(d)) => {
                let ord = c.to_lowercase().cmp(d.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                x.next();
                y.next();
            }
        }
    }
}

fn kind_of(st: &rustix::fs::Stat) -> Kind {
    match FileType::from_raw_mode(st.st_mode) {
        FileType::Directory => Kind::Folder,
        FileType::Symlink => Kind::Link,
        _ => Kind::File,
    }
}

fn describe(scope: &Scope, path: &RelPath, name: &str, st: rustix::fs::Stat) -> Entry {
    let mut kind = kind_of(&st);
    let (mut size, mut modified, mut outside) = (st.st_size as u64, st.st_mtime, false);
    if kind == Kind::Link {
        // A link inside the scope shows what it points to; one outside shows only that it is a link.
        match scope
            .open(path, OFlags::PATH)
            .and_then(|fd| Ok(rustix::fs::fstat(&fd)?))
        {
            Ok(target) => {
                kind = kind_of(&target);
                (size, modified) = (target.st_size as u64, target.st_mtime);
            }
            Err(_) => (size, outside) = (0, true),
        }
    }
    if kind == Kind::Folder {
        size = 0;
    }
    Entry {
        name: name.to_string(),
        kind,
        size,
        modified,
        mime: mime::guess(name),
        outside,
    }
}

/// One entry, for answers about a single file or folder.
pub fn entry(scope: &Scope, path: &RelPath) -> Result<Entry, FsError> {
    let st = scope.stat(path)?;
    Ok(describe(scope, path, path.name().unwrap_or(""), st))
}

pub fn list(scope: &Scope, path: &RelPath) -> Result<Vec<Entry>, FsError> {
    let dir = scope.open_dir(path)?;
    let mut entries = Vec::new();
    for item in rustix::fs::Dir::read_from(&dir)? {
        let item = item?;
        let raw = item.file_name().to_bytes();
        if raw == b"." || raw == b".." {
            continue;
        }
        let Some(name) = std::str::from_utf8(raw).ok() else {
            tracing::warn!(event = "file.name_not_utf8", name = %OsStr::from_bytes(raw).to_string_lossy());
            continue;
        };
        if name.starts_with(TEMP_PREFIX) {
            continue;
        }
        let Ok(st) = rustix::fs::statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW) else {
            continue;
        };
        if scope.data_id() == Some((st.st_dev, st.st_ino)) {
            continue;
        }
        let Ok(child) = path.join(name) else { continue };
        entries.push(describe(scope, &child, name, st));
    }
    entries.sort_by(|a, b| {
        let folder = |e: &Entry| e.kind != Kind::Folder;
        folder(a)
            .cmp(&folder(b))
            .then_with(|| natural_cmp(&a.name, &b.name))
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["b", "A", "file10", "file2", "File1", "z", "file02"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["A", "b", "File1", "file2", "file02", "file10", "z"]);
    }
}
