//! Trash folders: one `.cairn-trash` at the top of each mount inside the server root, each with a marker naming
//! this database. A folder of that name without our marker is someone else's: never used, hidden or cleaned.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

use rustix::fd::OwnedFd;
use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;

use crate::db::Db;
use crate::fs::FsError;
use crate::fs::mount::{mount_id, mount_points};
use crate::fs::scope::TRASH_NAME;

/// The marker file inside each trash folder of ours: one line, `cairn-trash <token>`.
pub const MARKER: &str = ".cairn-trash-marker";

const HINT: &str =
    "Rename this folder; Cairn keeps its trash there and will not touch a folder it did not make.";

pub fn id_fd(fd: &OwnedFd) -> (u64, u64) {
    let st = rustix::fs::fstat(fd).expect("fstat on an open folder");
    (st.st_dev, st.st_ino)
}

const DIR: OFlags = OFlags::DIRECTORY
    .union(OFlags::RDONLY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

fn up(fd: &OwnedFd) -> Result<OwnedFd, FsError> {
    Ok(rustix::fs::openat(fd, "..", DIR, Mode::empty())?)
}

/// The highest folder at or below `root` on the way up from `dir` that is on the same mount as `dir`. Without mount
/// ids (Linux before 5.8) that is `root`. Stops at `root`, never above it.
pub fn mount_top(root: &OwnedFd, dir: &OwnedFd) -> Result<OwnedFd, FsError> {
    let root_id = id_fd(root);
    let Some(mnt) = mount_id(dir) else {
        let points = fd_path(root).map(|r| mount_points(&r)).unwrap_or_default();
        return mount_top_by_path(root, dir, &points);
    };
    let mut cur = rustix::fs::openat(dir, ".", DIR, Mode::empty())?;
    for _ in 0..=crate::fs::MAX_DEPTH * 16 {
        if id_fd(&cur) == root_id {
            return Ok(cur);
        }
        let parent = up(&cur)?;
        if id_fd(&parent) == id_fd(&cur) || mount_id(&parent) != Some(mnt) {
            return Ok(cur);
        }
        cur = parent;
    }
    Err(FsError::TooDeep)
}

/// `mount_top` for kernels without mount ids: the deepest of `points` (mount points from mountinfo) that holds
/// `dir` and lies below `root`; `root` when none does. Bind mounts of one disk are told apart this way too.
pub fn mount_top_by_path(
    root: &OwnedFd,
    dir: &OwnedFd,
    points: &[PathBuf],
) -> Result<OwnedFd, FsError> {
    let (Some(base), Some(here)) = (fd_path(root), fd_path(dir)) else {
        return Ok(rustix::fs::openat(root, ".", DIR, Mode::empty())?);
    };
    let best = points
        .iter()
        .filter(|p| p.starts_with(&base) && **p != base && here.starts_with(p))
        .max_by_key(|p| p.components().count());
    match best {
        Some(p) => Ok(rustix::fs::open(p, DIR, Mode::empty())?),
        None => Ok(rustix::fs::openat(root, ".", DIR, Mode::empty())?),
    }
}

fn fd_path(fd: &OwnedFd) -> Option<PathBuf> {
    std::fs::read_link(format!(
        "/proc/self/fd/{}",
        rustix::fd::AsRawFd::as_raw_fd(fd)
    ))
    .ok()
}

#[derive(Default)]
struct Inner {
    /// Stored path of each trash folder of ours (`/.cairn-trash`, `/photos/.cairn-trash`) and its id.
    ours: Vec<(String, (u64, u64))>,
    ids: Arc<[(u64, u64)]>,
    /// Mount tops whose `.cairn-trash` is not ours (`/`, `/photos`).
    taken: Vec<String>,
}

/// Every trash folder Cairn knows of under the server root.
pub struct Trashes {
    root: PathBuf,
    inner: Mutex<Inner>,
    /// Moves into and out of the trash hold it shared; reconcile and the purge hold it alone, so they never see a
    /// row whose item is between its row and its rename.
    moves: RwLock<()>,
}

enum Found {
    Ours((u64, u64)),
    Taken,
    Missing,
}

fn read_marker(trash: &OwnedFd) -> Option<String> {
    let fd = rustix::fs::openat(
        trash,
        MARKER,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    let mut text = String::new();
    std::fs::File::from(fd)
        .take(256)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

fn inspect(top: &OwnedFd, token: Option<&str>) -> Result<Found, FsError> {
    let trash = match rustix::fs::openat(top, TRASH_NAME, DIR, Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(Found::Missing),
        // A file or a link by that name: not ours.
        Err(Errno::NOTDIR | Errno::LOOP) => return Ok(Found::Taken),
        Err(e) => return Err(e.into()),
    };
    match (read_marker(&trash), token) {
        (Some(text), Some(t)) if text == format!("cairn-trash {t}\n") => {
            Ok(Found::Ours(id_fd(&trash)))
        }
        _ => Ok(Found::Taken),
    }
}

impl Trashes {
    /// Looks at the server root and every mount below it. Never fails: a folder it cannot read is simply not known
    /// yet. Logs each `.cairn-trash` that is not ours.
    pub fn open(root: &Path, db: &Db) -> Trashes {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let t = Trashes {
            root,
            inner: Mutex::new(Inner::default()),
            moves: RwLock::new(()),
        };
        t.refresh(db);
        t
    }

    /// Held while an item moves into or out of the trash, or is removed from it.
    pub fn moving(&self) -> RwLockReadGuard<'_, ()> {
        self.moves.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Held by reconcile and the purge: no move is in flight meanwhile.
    pub fn upkeep(&self) -> RwLockWriteGuard<'_, ()> {
        self.moves.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Looks again at every mount top, for the startup check and each purge.
    pub fn refresh(&self, db: &Db) {
        let token = crate::trash::existing_token(db);
        let mut tops = vec![self.root.clone()];
        tops.extend(mount_points(&self.root));
        for top in tops {
            let Ok(fd) = rustix::fs::open(&top, DIR, Mode::empty()) else {
                continue;
            };
            let Ok(found) = inspect(&fd, token.as_deref()) else {
                continue;
            };
            let stored = self.stored(&fd);
            match found {
                Found::Ours(id) => self.record(&stored, id),
                Found::Taken => self.mark_taken(&stored),
                Found::Missing => {}
            }
        }
    }

    /// `top` as stored: `/` for the root, `/photos` below it.
    fn stored(&self, top: &OwnedFd) -> String {
        let rel = fd_path(top)
            .and_then(|p| p.strip_prefix(&self.root).ok().map(Path::to_path_buf))
            .unwrap_or_default();
        format!("/{}", rel.to_string_lossy())
    }

    fn trash_path(top: &str) -> String {
        format!("{}/{TRASH_NAME}", top.trim_end_matches('/'))
    }

    fn record(&self, top: &str, id: (u64, u64)) {
        let mut inner = self.inner.lock().expect("trashes");
        inner.taken.retain(|t| t != top);
        if !inner.ours.iter().any(|(_, i)| *i == id) {
            inner.ours.push((Self::trash_path(top), id));
            inner.ids = inner.ours.iter().map(|(_, i)| *i).collect();
        }
    }

    fn mark_taken(&self, top: &str) {
        let mut inner = self.inner.lock().expect("trashes");
        if !inner.taken.iter().any(|t| t == top) {
            tracing::warn!(
                event = "startup.trash_folder_taken",
                folder = %Self::trash_path(top),
                hint = HINT
            );
            inner.taken.push(top.to_string());
            inner.taken.sort();
        }
    }

    /// The trash folder of the mount whose top is `top`, made with its marker on first use, and its stored path.
    pub fn for_dir(&self, db: &Db, top: &OwnedFd) -> Result<(OwnedFd, String), FsError> {
        let stored = self.stored(top);
        for _ in 0..2 {
            let token = crate::trash::marker_token(db)
                .map_err(|e| FsError::Io(std::io::Error::other(e)))?;
            match inspect(top, Some(&token))? {
                Found::Ours(id) => {
                    self.record(&stored, id);
                    let fd = rustix::fs::openat(top, TRASH_NAME, DIR, Mode::empty())?;
                    return Ok((fd, Self::trash_path(&stored)));
                }
                Found::Taken => {
                    self.mark_taken(&stored);
                    return Err(FsError::TrashTaken(stored));
                }
                Found::Missing => {
                    match rustix::fs::mkdirat(top, TRASH_NAME, Mode::from_raw_mode(0o700)) {
                        Ok(()) => {
                            let fd = rustix::fs::openat(top, TRASH_NAME, DIR, Mode::empty())?;
                            rustix::fs::fchmod(&fd, Mode::from_raw_mode(0o700))?;
                            let marker = rustix::fs::openat(
                                &fd,
                                MARKER,
                                OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::CLOEXEC,
                                Mode::from_raw_mode(0o600),
                            )?;
                            std::fs::File::from(marker)
                                .write_all(format!("cairn-trash {token}\n").as_bytes())?;
                        }
                        // Made by another request a moment ago: look again.
                        Err(Errno::EXIST) => {}
                        Err(e) => return Err(e.into()),
                    }
                }
            }
        }
        Err(FsError::TrashTaken(stored))
    }

    /// Device and inode of every trash folder of ours, for `Reserved`.
    pub fn ids(&self) -> Arc<[(u64, u64)]> {
        self.inner.lock().expect("trashes").ids.clone()
    }

    /// Stored paths of every trash folder of ours.
    pub fn ours(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("trashes")
            .ours
            .iter()
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// Mount tops whose `.cairn-trash` is not ours.
    pub fn taken(&self) -> Vec<String> {
        self.inner.lock().expect("trashes").taken.clone()
    }

    /// The server root, as the trash sees it.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::Path;

    use super::*;
    use crate::db::Db;
    use crate::fs::mount::fake_mount_id;

    fn id(p: &Path) -> (u64, u64) {
        let m = std::fs::symlink_metadata(p).unwrap();
        (m.dev(), m.ino())
    }

    fn dir(p: &Path) -> OwnedFd {
        rustix::fs::open(
            p,
            OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap()
    }

    /// root/{docs, photos/2026}, with `photos` its own mount.
    fn tree() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("srv");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(root.join("photos/2026")).unwrap();
        fake_mount_id(t.path(), 8000);
        fake_mount_id(&root, 8001);
        fake_mount_id(&root.join("docs"), 8001);
        fake_mount_id(&root.join("photos"), 8002);
        fake_mount_id(&root.join("photos/2026"), 8002);
        t
    }

    #[test]
    fn mount_top_walks_to_mount() {
        let t = tree();
        let root = t.path().join("srv");
        let r = dir(&root);
        let top = mount_top(&r, &dir(&root.join("photos/2026"))).unwrap();
        assert_eq!(id_fd(&top), id(&root.join("photos")));
        let top = mount_top(&r, &dir(&root.join("docs"))).unwrap();
        assert_eq!(id_fd(&top), id(&root));
    }

    /// Without mount ids (Linux before 5.8), the mount points from mountinfo decide.
    #[test]
    fn mount_top_without_mount_ids_uses_mount_points() {
        let t = tree();
        let root = t.path().join("srv");
        let r = dir(&root);
        let points = vec![std::fs::canonicalize(root.join("photos")).unwrap()];
        let top = mount_top_by_path(&r, &dir(&root.join("photos/2026")), &points).unwrap();
        assert_eq!(id_fd(&top), id(&root.join("photos")));
        let top = mount_top_by_path(&r, &dir(&root.join("docs")), &points).unwrap();
        assert_eq!(id_fd(&top), id(&root));
        // A mount point outside the root never counts.
        let outside = vec![std::fs::canonicalize(t.path()).unwrap()];
        let top = mount_top_by_path(&r, &dir(&root.join("photos/2026")), &outside).unwrap();
        assert_eq!(id_fd(&top), id(&root));
    }

    #[test]
    fn creates_with_marker_and_mode() {
        let t = tree();
        let root = t.path().join("srv");
        let db = Db::open_in_memory();
        let trashes = Trashes::open(&root, &db);
        assert!(trashes.ids().is_empty());
        let r = dir(&root);
        let (_, stored) = trashes
            .for_dir(&db, &mount_top(&r, &dir(&root.join("docs"))).unwrap())
            .unwrap();
        assert_eq!(stored, "/.cairn-trash");
        let (_, stored) = trashes
            .for_dir(
                &db,
                &mount_top(&r, &dir(&root.join("photos/2026"))).unwrap(),
            )
            .unwrap();
        assert_eq!(stored, "/photos/.cairn-trash");
        for folder in [root.join(".cairn-trash"), root.join("photos/.cairn-trash")] {
            let mode = std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
            let token = crate::trash::marker_token(&db).unwrap();
            assert_eq!(
                std::fs::read_to_string(folder.join(MARKER)).unwrap(),
                format!("cairn-trash {token}\n")
            );
            assert!(trashes.ids().contains(&id(&folder)));
        }
        // Opening again finds both as ours.
        let again = Trashes::open(&root, &db);
        assert!(again.taken().is_empty());
    }

    #[test]
    fn foreign_folder_is_taken() {
        let t = tree();
        let root = t.path().join("srv");
        std::fs::create_dir_all(root.join(".cairn-trash")).unwrap();
        std::fs::write(root.join(".cairn-trash/notes.txt"), "mine").unwrap();
        let db = Db::open_in_memory();
        let trashes = Trashes::open(&root, &db);
        assert_eq!(trashes.taken(), vec!["/".to_string()]);
        let r = dir(&root);
        assert!(matches!(
            trashes.for_dir(&db, &mount_top(&r, &dir(&root.join("docs"))).unwrap()),
            Err(FsError::TrashTaken(f)) if f == "/"
        ));
        assert!(trashes.ids().is_empty());
        assert_eq!(
            std::fs::read_to_string(root.join(".cairn-trash/notes.txt")).unwrap(),
            "mine"
        );
        assert!(!root.join(".cairn-trash").join(MARKER).exists());
        // Still an ordinary folder to a scope-`/` user.
        let scope = crate::fs::scope::Scope::new(&root, "/", crate::fs::scope::Reserved::default())
            .unwrap();
        let names: Vec<String> =
            crate::fs::list::list(&scope, &crate::fs::path::RelPath::parse("").unwrap())
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
        assert!(names.contains(&".cairn-trash".to_string()));
    }

    #[test]
    fn taken_is_per_mount() {
        let t = tree();
        let root = t.path().join("srv");
        std::fs::create_dir_all(root.join(".cairn-trash")).unwrap();
        let db = Db::open_in_memory();
        let trashes = Trashes::open(&root, &db);
        let r = dir(&root);
        let (_, stored) = trashes
            .for_dir(
                &db,
                &mount_top(&r, &dir(&root.join("photos/2026"))).unwrap(),
            )
            .unwrap();
        assert_eq!(stored, "/photos/.cairn-trash");
        assert_eq!(trashes.taken(), vec!["/".to_string()]);
    }

    #[test]
    fn other_database_marker_is_taken() {
        let t = tree();
        let root = t.path().join("srv");
        std::fs::create_dir_all(root.join(".cairn-trash")).unwrap();
        std::fs::write(
            root.join(".cairn-trash").join(MARKER),
            "cairn-trash AAAAAAAAAAAAAAAAAAAAAA\n",
        )
        .unwrap();
        let db = Db::open_in_memory();
        crate::trash::marker_token(&db).unwrap();
        let trashes = Trashes::open(&root, &db);
        assert_eq!(trashes.taken(), vec!["/".to_string()]);
    }
}
