//! The user's scope: every path a client sends is opened beneath it, never outside, never into the data folder.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags, ResolveFlags};
use rustix::io::Errno;

use super::FsError;
use super::path::RelPath;

pub struct Scope {
    root: OwnedFd,
    data_id: Option<(u64, u64)>,
    mode: Resolve,
}

enum Resolve {
    /// Linux 5.6 and later: `openat2` keeps every lookup beneath the scope, even through symlinks.
    Openat2,
    /// Older kernels: walk one folder at a time with `O_NOFOLLOW`, refusing every symlink.
    Fallback,
}

const RESOLVE: ResolveFlags = ResolveFlags::BENEATH.union(ResolveFlags::NO_MAGICLINKS);

fn openat2_available(fd: &OwnedFd) -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        !matches!(
            rustix::fs::openat2(
                fd,
                ".",
                OFlags::PATH | OFlags::CLOEXEC,
                Mode::empty(),
                RESOLVE
            ),
            Err(Errno::NOSYS)
        )
    })
}

fn id_of(st: &rustix::fs::Stat) -> (u64, u64) {
    (st.st_dev, st.st_ino)
}

fn fd_path(fd: &OwnedFd) -> std::io::Result<PathBuf> {
    std::fs::read_link(format!(
        "/proc/self/fd/{}",
        rustix::fd::AsRawFd::as_raw_fd(fd)
    ))
}

impl Scope {
    /// `scope` is the user's folder as stored, like `/photos`; it was validated when it was saved.
    pub fn new(
        server_root: &Path,
        scope: &str,
        data_id: Option<(u64, u64)>,
    ) -> Result<Scope, FsError> {
        let path = server_root.join(scope.trim_start_matches('/'));
        let root = rustix::fs::openat(
            CWD,
            &path,
            OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let mode = if openat2_available(&root) {
            Resolve::Openat2
        } else {
            Resolve::Fallback
        };
        Ok(Scope {
            root,
            data_id,
            mode,
        })
    }

    /// Forces the older-kernel path, for tests.
    pub fn with_fallback(self) -> Scope {
        Scope {
            mode: Resolve::Fallback,
            ..self
        }
    }

    fn open_raw(&self, path: &RelPath, flags: OFlags) -> Result<OwnedFd, FsError> {
        let flags = flags | OFlags::CLOEXEC;
        // openat2 refuses a mode unless the call creates a file.
        let mode = if flags.contains(OFlags::CREATE) {
            Mode::from_raw_mode(0o644)
        } else {
            Mode::empty()
        };
        match self.mode {
            Resolve::Openat2 => Ok(rustix::fs::openat2(
                &self.root,
                path.to_os(),
                flags,
                mode,
                RESOLVE,
            )?),
            Resolve::Fallback => self.open_walk(path, flags, mode),
        }
    }

    /// One folder at a time from the scope's descriptor, never following a link, checking each folder against the
    /// data folder as it is opened. Nothing is looked up by path twice, so a rename racing with the request cannot
    /// redirect it.
    fn open_walk(&self, path: &RelPath, flags: OFlags, mode: Mode) -> Result<OwnedFd, FsError> {
        let dir_flags = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::RDONLY | OFlags::CLOEXEC;
        let mut cur = rustix::fs::openat(&self.root, ".", dir_flags, Mode::empty())?;
        let Some((last, folders)) = path.segments().split_last() else {
            return Ok(rustix::fs::openat(&self.root, ".", flags, mode)?);
        };
        for seg in folders {
            let name = OsStr::from_bytes(seg);
            let next = match rustix::fs::openat(&cur, name, dir_flags, Mode::empty()) {
                Ok(fd) => fd,
                // O_DIRECTORY with O_NOFOLLOW on a link answers ENOTDIR; say what it is.
                Err(Errno::NOTDIR)
                    if rustix::fs::statat(&cur, name, AtFlags::SYMLINK_NOFOLLOW).is_ok_and(
                        |st| FileType::from_raw_mode(st.st_mode) == FileType::Symlink,
                    ) =>
                {
                    return Err(FsError::Outside);
                }
                Err(e) => return Err(e.into()),
            };
            if Some(id_of(&rustix::fs::fstat(&next)?)) == self.data_id {
                return Err(FsError::NotFound);
            }
            cur = next;
        }
        let fd = rustix::fs::openat(
            &cur,
            OsStr::from_bytes(last),
            flags | OFlags::NOFOLLOW,
            mode,
        )?;
        // With O_PATH, O_NOFOLLOW opens the link itself instead of failing; refuse it the same way.
        if FileType::from_raw_mode(rustix::fs::fstat(&fd)?.st_mode) == FileType::Symlink {
            return Err(FsError::Outside);
        }
        Ok(fd)
    }

    /// Refuses what was opened when it is the data folder or lies inside it, also when reached through a symlink
    /// or another mount of the same folder. It checks the object actually opened, not the path again.
    fn check_data(&self, fd: &OwnedFd, path: &RelPath) -> Result<(), FsError> {
        let Some(data) = self.data_id else {
            return Ok(());
        };
        if id_of(&rustix::fs::fstat(fd)?) == data {
            return Err(FsError::NotFound);
        }
        let (Ok(real), Ok(base)) = (fd_path(fd), fd_path(&self.root)) else {
            // No /proc: check each folder named on the path instead.
            for n in 0..path.len() {
                let fd = self.open_raw(&path.prefix(n), OFlags::PATH)?;
                if id_of(&rustix::fs::fstat(&fd)?) == data {
                    return Err(FsError::NotFound);
                }
            }
            return Ok(());
        };
        let mut folder = real.parent();
        while let Some(dir) = folder {
            if !dir.starts_with(&base) {
                break;
            }
            if let Ok(st) = rustix::fs::stat(dir)
                && id_of(&st) == data
            {
                return Err(FsError::NotFound);
            }
            if dir == base {
                break;
            }
            folder = dir.parent();
        }
        Ok(())
    }

    /// Opens `path`, following symlinks that stay inside the scope.
    pub fn open(&self, path: &RelPath, flags: OFlags) -> Result<OwnedFd, FsError> {
        let fd = self.open_raw(path, flags)?;
        self.check_data(&fd, path)?;
        Ok(fd)
    }

    pub fn open_dir(&self, path: &RelPath) -> Result<OwnedFd, FsError> {
        self.open(path, OFlags::DIRECTORY | OFlags::RDONLY)
    }

    /// The parent folder and the final name, for `*at` calls that act on the name itself (never its link target).
    pub fn parent(&self, path: &RelPath) -> Result<(OwnedFd, OsString), FsError> {
        let (Some(parent), Some(name)) = (path.parent(), path.name_bytes()) else {
            return Err(FsError::ScopeRoot);
        };
        let dir = self.open_dir(&parent)?;
        if let (Some(data), Ok(st)) = (
            self.data_id,
            rustix::fs::statat(&dir, OsStr::from_bytes(name), AtFlags::SYMLINK_NOFOLLOW),
        ) && id_of(&st) == data
        {
            return Err(FsError::NotFound);
        }
        Ok((dir, OsStr::from_bytes(name).to_os_string()))
    }

    /// Without following a final symlink.
    pub fn stat(&self, path: &RelPath) -> Result<rustix::fs::Stat, FsError> {
        if path.is_root() {
            return Ok(rustix::fs::fstat(&self.root)?);
        }
        let (dir, name) = self.parent(path)?;
        Ok(rustix::fs::statat(
            &dir,
            name.as_os_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        )?)
    }

    /// Whether `path`, a symlink, resolves to something inside the scope.
    pub fn link_inside(&self, path: &RelPath) -> bool {
        self.open(path, OFlags::PATH).is_ok()
    }

    /// Device and inode of the scope folder itself.
    pub fn root_id(&self) -> Result<(u64, u64), FsError> {
        Ok(id_of(&rustix::fs::fstat(&self.root)?))
    }

    pub fn data_id(&self) -> Option<(u64, u64)> {
        self.data_id
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    /// root/a is the scope: f.txt, sub/g.txt, in -> sub/g.txt, up -> ../b, abs -> /etc/passwd, .db (the data folder).
    struct Tree {
        dir: tempfile::TempDir,
        data_id: (u64, u64),
    }

    fn tree() -> Tree {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        std::fs::create_dir_all(a.join("sub")).unwrap();
        std::fs::create_dir_all(a.join(".db")).unwrap();
        std::fs::create_dir_all(dir.path().join("b")).unwrap();
        std::fs::write(a.join("f.txt"), "f").unwrap();
        std::fs::write(a.join("sub/g.txt"), "g").unwrap();
        std::fs::write(dir.path().join("b/secret.txt"), "s").unwrap();
        symlink("sub/g.txt", a.join("in")).unwrap();
        symlink("../b", a.join("up")).unwrap();
        symlink("/etc/passwd", a.join("abs")).unwrap();
        let st = std::fs::metadata(a.join(".db")).unwrap();
        use std::os::unix::fs::MetadataExt;
        Tree {
            data_id: (st.dev(), st.ino()),
            dir,
        }
    }

    fn scopes(t: &Tree) -> [Scope; 2] {
        let open = || Scope::new(t.dir.path(), "/a", Some(t.data_id)).unwrap();
        [open(), open().with_fallback()]
    }

    fn p(s: &str) -> RelPath {
        RelPath::parse(s).unwrap()
    }

    fn read(scope: &Scope, path: &str) -> Result<String, FsError> {
        let fd = scope.open(&p(path), OFlags::RDONLY)?;
        Ok(std::io::read_to_string(std::fs::File::from(fd)).unwrap())
    }

    #[test]
    fn opens_file() {
        let t = tree();
        for s in scopes(&t) {
            assert_eq!(read(&s, "f.txt").unwrap(), "f");
            assert_eq!(read(&s, "sub/g.txt").unwrap(), "g");
        }
    }

    #[test]
    fn follows_inside_link() {
        let t = tree();
        let [modern, _] = scopes(&t);
        assert_eq!(read(&modern, "in").unwrap(), "g");
    }

    #[test]
    fn fallback_refuses_every_link() {
        let t = tree();
        let [_, old] = scopes(&t);
        assert!(matches!(read(&old, "in"), Err(FsError::Outside)));
    }

    #[test]
    fn refuses_link_to_sibling_scope() {
        let t = tree();
        for s in scopes(&t) {
            assert!(matches!(read(&s, "up/secret.txt"), Err(FsError::Outside)));
        }
    }

    #[test]
    fn refuses_absolute_link() {
        let t = tree();
        for s in scopes(&t) {
            assert!(matches!(read(&s, "abs"), Err(FsError::Outside)));
        }
    }

    #[test]
    fn data_folder_is_not_found() {
        let t = tree();
        std::fs::write(t.dir.path().join("a/.db/x"), "db").unwrap();
        for s in scopes(&t) {
            assert!(matches!(s.open_dir(&p(".db")), Err(FsError::NotFound)));
            assert!(matches!(read(&s, ".db/x"), Err(FsError::NotFound)));
            assert!(matches!(s.parent(&p(".db")), Err(FsError::NotFound)));
            assert!(matches!(s.stat(&p(".db")), Err(FsError::NotFound)));
        }
    }

    #[test]
    fn parent_does_not_follow_final_link() {
        let t = tree();
        for s in scopes(&t) {
            let (dir, name) = s.parent(&p("up")).unwrap();
            assert_eq!(name, "up");
            let st = rustix::fs::statat(&dir, "up", AtFlags::SYMLINK_NOFOLLOW).unwrap();
            assert_eq!(st.st_mode & 0o170000, 0o120000, "the link itself");
        }
    }

    #[test]
    fn root_has_no_parent() {
        let t = tree();
        for s in scopes(&t) {
            assert!(matches!(s.parent(&p("")), Err(FsError::ScopeRoot)));
        }
    }

    #[test]
    fn link_inside_marks() {
        let t = tree();
        let [modern, old] = scopes(&t);
        assert!(modern.link_inside(&p("in")));
        assert!(!modern.link_inside(&p("up")));
        assert!(!modern.link_inside(&p("abs")));
        assert!(!old.link_inside(&p("in")));
    }

    #[test]
    fn missing_is_not_found() {
        let t = tree();
        for s in scopes(&t) {
            assert!(matches!(read(&s, "nope.txt"), Err(FsError::NotFound)));
            assert!(matches!(s.stat(&p("nope")), Err(FsError::NotFound)));
        }
    }

    #[test]
    fn link_to_a_file_in_the_data_folder_is_not_found() {
        let t = tree();
        std::fs::write(t.dir.path().join("a/.db/cairn.db"), "db").unwrap();
        symlink(".db/cairn.db", t.dir.path().join("a/db-link")).unwrap();
        symlink(".db", t.dir.path().join("a/db-dir-link")).unwrap();
        let [modern, _] = scopes(&t);
        assert!(matches!(read(&modern, "db-link"), Err(FsError::NotFound)));
        assert!(matches!(
            modern.open_dir(&p("db-dir-link")),
            Err(FsError::NotFound)
        ));
        assert!(!modern.link_inside(&p("db-link")));
    }
}
