//! The user's scope: every path a client sends is opened beneath it, never outside, never into the data folder.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags, ResolveFlags};
use rustix::io::Errno;

use super::FsError;
use super::path::RelPath;

pub struct Scope {
    root: OwnedFd,
    reserved: Reserved,
    mode: Resolve,
}

/// Folders no path from a client may reach, by device and inode, and the server root they are reserved in.
#[derive(Clone, Default, Debug)]
pub struct Reserved {
    /// `CAIRN_DATA`.
    pub data: Option<(u64, u64)>,
    /// Every trash folder of ours.
    pub trash: Arc<[(u64, u64)]>,
    /// `CAIRN_ROOT` itself, where the trash folder's name is kept free.
    pub root: Option<(u64, u64)>,
}

impl Reserved {
    pub fn data_only(data: Option<(u64, u64)>) -> Reserved {
        Reserved {
            data,
            ..Reserved::default()
        }
    }

    pub fn has(&self, id: (u64, u64)) -> bool {
        self.data == Some(id) || self.trash.contains(&id)
    }
}

/// The name of the trash folder at the top of each mount.
pub const TRASH_NAME: &str = ".cairn-trash";

#[derive(Clone, Copy)]
enum Resolve {
    /// Linux 5.6 and later: `openat2` keeps every lookup beneath the scope, even through symlinks.
    Openat2,
    /// `openat2` that refuses every link, for a path someone else may change: see `Scope::no_links`.
    NoLinks,
    /// Older kernels: walk one folder at a time with `O_NOFOLLOW`, refusing every symlink.
    Fallback,
}

const RESOLVE: ResolveFlags = ResolveFlags::BENEATH.union(ResolveFlags::NO_MAGICLINKS);
const NO_LINKS: ResolveFlags = RESOLVE.union(ResolveFlags::NO_SYMLINKS);

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

const FOLDER: OFlags = OFlags::DIRECTORY
    .union(OFlags::RDONLY)
    .union(OFlags::CLOEXEC);

/// The folder a stored path names, reached from `base` through real folders only.
fn open_stored(
    base: OwnedFd,
    rel: &RelPath,
    reserved: Reserved,
    mode: Resolve,
) -> Result<OwnedFd, FsError> {
    match mode {
        Resolve::Fallback => Scope {
            root: base,
            reserved,
            mode,
        }
        .open_walk(rel, FOLDER, Mode::empty()),
        Resolve::Openat2 | Resolve::NoLinks => Ok(rustix::fs::openat2(
            &base,
            rel.to_os(),
            FOLDER,
            Mode::empty(),
            NO_LINKS,
        )?),
    }
}

impl Scope {
    /// `scope` is the user's folder as stored, like `/photos`; it was validated when it was saved.
    pub fn new(server_root: &Path, scope: &str, reserved: Reserved) -> Result<Scope, FsError> {
        let base = rustix::fs::openat(CWD, server_root, FOLDER, Mode::empty())?;
        let rel = RelPath::parse(scope)?;
        let mode = if openat2_available(&base) {
            Resolve::Openat2
        } else {
            Resolve::Fallback
        };
        let root = open_stored(base, &rel, reserved.clone(), mode)?;
        Ok(Scope {
            root,
            reserved,
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

    /// The same folder, where no path may pass a link at all. For the stored path of a share link or a grant.
    pub fn no_links(self) -> Scope {
        let mode = match self.mode {
            Resolve::Fallback => Resolve::Fallback,
            Resolve::Openat2 | Resolve::NoLinks => Resolve::NoLinks,
        };
        Scope { mode, ..self }
    }

    /// Links inside the folder work again, always beneath it.
    pub fn follow_links(self) -> Scope {
        let mode = match self.mode {
            Resolve::Fallback => Resolve::Fallback,
            Resolve::Openat2 | Resolve::NoLinks => Resolve::Openat2,
        };
        Scope { mode, ..self }
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
            Resolve::NoLinks => Ok(rustix::fs::openat2(
                &self.root,
                path.to_os(),
                flags,
                mode,
                NO_LINKS,
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
            if self.reserved.has(id_of(&rustix::fs::fstat(&next)?)) {
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
        let r = &self.reserved;
        if r.data.is_none() && r.trash.is_empty() {
            return Ok(());
        }
        if r.has(id_of(&rustix::fs::fstat(fd)?)) {
            return Err(FsError::NotFound);
        }
        let (Ok(real), Ok(base)) = (fd_path(fd), fd_path(&self.root)) else {
            // No /proc: check each folder named on the path instead.
            for n in 0..path.len() {
                let fd = self.open_raw(&path.prefix(n), OFlags::PATH)?;
                if r.has(id_of(&rustix::fs::fstat(&fd)?)) {
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
                && r.has(id_of(&st))
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

    /// The folder at `path` as a scope of its own: nothing opened through it can reach outside that folder.
    pub fn sub(&self, path: &RelPath) -> Result<Scope, FsError> {
        let root = self.open_dir(path)?;
        Ok(Scope {
            root,
            reserved: self.reserved.clone(),
            mode: self.mode,
        })
    }

    /// Where an opened folder really lies beneath this scope, whatever links led to it.
    pub fn real_path(&self, dir: &OwnedFd) -> Result<RelPath, FsError> {
        let (dir, base) = (fd_path(dir)?, fd_path(&self.root)?);
        let rel = dir.strip_prefix(&base).map_err(|_| FsError::Outside)?;
        rel.iter().try_fold(RelPath::parse("")?, |path, seg| {
            path.join_bytes(seg.as_bytes())
        })
    }

    /// The parent folder and the final name, for `*at` calls that act on the name itself (never its link target).
    pub fn parent(&self, path: &RelPath) -> Result<(OwnedFd, OsString), FsError> {
        let (Some(parent), Some(name)) = (path.parent(), path.name_bytes()) else {
            return Err(FsError::ScopeRoot);
        };
        let dir = self.open_dir(&parent)?;
        if let Ok(st) = rustix::fs::statat(&dir, OsStr::from_bytes(name), AtFlags::SYMLINK_NOFOLLOW)
            && self.reserved.has(id_of(&st))
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

    /// Whether the link at `path` resolves outside the scope. A link that leads nowhere does not, a loop of links
    /// counts as leaving, and where links are not followed at all none can lead anywhere.
    pub fn link_leaves(&self, path: &RelPath) -> bool {
        matches!(self.mode, Resolve::Openat2)
            && matches!(self.open(path, OFlags::PATH), Err(FsError::Outside))
    }

    /// Used and total bytes of the filesystem holding the scope.
    pub fn space(&self) -> Result<(u64, u64), FsError> {
        let v = rustix::fs::fstatvfs(&self.root)?;
        let total = v.f_blocks.saturating_mul(v.f_frsize);
        let used = v
            .f_blocks
            .saturating_sub(v.f_bfree)
            .saturating_mul(v.f_frsize);
        Ok((used, total))
    }

    /// Device and inode of the scope folder itself.
    pub fn root_id(&self) -> Result<(u64, u64), FsError> {
        Ok(id_of(&rustix::fs::fstat(&self.root)?))
    }

    /// Whether this device and inode is the data folder or a trash folder.
    pub fn is_reserved(&self, id: (u64, u64)) -> bool {
        self.reserved.has(id)
    }

    /// Refuses a name a new entry in `dir` may not take: upload temporaries anywhere, and the trash folder's name
    /// in the server root or at the top of a mount inside it.
    pub fn check_new_name(&self, dir: &OwnedFd, name: &OsStr) -> Result<(), FsError> {
        let raw = name.as_bytes();
        if raw.starts_with(super::list::TEMP_PREFIX.as_bytes()) {
            return Err(FsError::InvalidPath);
        }
        if raw != TRASH_NAME.as_bytes() {
            return Ok(());
        }
        if self.reserved.root == Some(id_of(&rustix::fs::fstat(dir)?)) {
            return Err(FsError::Reserved);
        }
        let up = rustix::fs::openat(
            dir,
            "..",
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let (here, above) = (super::mount::mount_id(dir), super::mount::mount_id(&up));
        if here.is_some() && above.is_some() {
            return if here != above {
                Err(FsError::Reserved)
            } else {
                Ok(())
            };
        }
        // No mount ids (Linux before 5.8): a folder named as a mount point in mountinfo is a mount top.
        if let Ok(path) = fd_path(dir)
            && super::mount::mount_points(Path::new("/")).contains(&path)
        {
            return Err(FsError::Reserved);
        }
        Ok(())
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
        scopes_with(t, Reserved::data_only(Some(t.data_id)))
    }

    fn scopes_with(t: &Tree, reserved: Reserved) -> [Scope; 2] {
        let open = || Scope::new(t.dir.path(), "/a", reserved.clone()).unwrap();
        [open(), open().with_fallback()]
    }

    #[test]
    fn stored_scope_never_passes_a_link() {
        let t = tree();
        let root = t.dir.path();
        symlink("a", root.join("to-a")).unwrap();
        std::fs::create_dir_all(root.join("via")).unwrap();
        symlink("../a", root.join("via/link")).unwrap();
        for stored in ["/to-a", "/via/link", "/via/link/sub", "/a/up"] {
            assert!(
                Scope::new(root, stored, Reserved::default()).is_err(),
                "{stored}"
            );
        }
        assert!(Scope::new(root, "/a/sub", Reserved::default()).is_ok());
        assert!(Scope::new(root, "/", Reserved::default()).is_ok());
    }

    #[test]
    fn the_server_root_itself_may_be_a_link() {
        let t = tree();
        let holder = tempfile::tempdir().unwrap();
        let root = holder.path().join("root");
        symlink(t.dir.path(), &root).unwrap();
        assert!(Scope::new(&root, "/", Reserved::default()).is_ok());
        assert!(Scope::new(&root, "/a/sub", Reserved::default()).is_ok());
    }

    #[test]
    fn stored_scope_never_passes_a_link_on_older_kernels() {
        let t = tree();
        let root = t.dir.path();
        symlink("a", root.join("to-a")).unwrap();
        let open = |stored: &str| {
            let rel = RelPath::parse(stored).unwrap();
            let base = rustix::fs::openat(CWD, root, FOLDER, Mode::empty()).unwrap();
            open_stored(base, &rel, Reserved::default(), Resolve::Fallback)
        };
        assert!(matches!(open("/to-a"), Err(FsError::NotAFolder)));
        assert!(matches!(open("/to-a/sub"), Err(FsError::Outside)));
        assert!(open("/a/sub").is_ok());
        assert!(open("/").is_ok());
    }

    #[test]
    fn strict_scope_follows_no_link_and_a_scope_below_it_does() {
        let t = tree();
        let a = t.dir.path().join("a");
        symlink("sub", a.join("dir")).unwrap();
        for s in scopes(&t) {
            let strict = s.no_links();
            assert!(matches!(
                strict.open(&p("in"), OFlags::RDONLY),
                Err(FsError::Outside)
            ));
            assert!(matches!(
                strict.open(&p("dir/g.txt"), OFlags::RDONLY),
                Err(FsError::Outside)
            ));
            assert!(strict.open(&p("sub/g.txt"), OFlags::RDONLY).is_ok());
            assert!(strict.sub(&p("dir")).is_err());
        }
        std::fs::write(a.join("sub/h.txt"), "h").unwrap();
        symlink("h.txt", a.join("sub/to-h")).unwrap();
        let [modern, _] = scopes(&t);
        let inner = modern.no_links().sub(&p("sub")).unwrap().follow_links();
        assert!(inner.open(&p("to-h"), OFlags::RDONLY).is_ok());
    }

    #[test]
    fn a_link_leaves_only_when_it_resolves_outside() {
        let t = tree();
        let a = t.dir.path().join("a");
        symlink("missing", a.join("dangling")).unwrap();
        let [modern, old] = scopes(&t);
        assert!(modern.link_leaves(&p("up")));
        assert!(modern.link_leaves(&p("abs")));
        assert!(!modern.link_leaves(&p("in")));
        assert!(!modern.link_leaves(&p("dangling")));
        // Without openat2 no link is ever followed, so none can lead anywhere.
        assert!(!old.link_leaves(&p("up")));
    }

    #[test]
    fn real_path_is_where_a_folder_lies_beneath_the_scope() {
        let t = tree();
        let [modern, _] = scopes(&t);
        symlink("sub", t.dir.path().join("a/dir")).unwrap();
        for path in ["sub", "dir"] {
            let dir = modern.open_dir(&p(path)).unwrap();
            assert_eq!(modern.real_path(&dir).unwrap().to_string(), "sub");
        }
        let root = modern.open_dir(&p("")).unwrap();
        assert!(modern.real_path(&root).unwrap().is_root());
    }

    fn id_of_path(p: &std::path::Path) -> (u64, u64) {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::symlink_metadata(p).unwrap();
        (m.dev(), m.ino())
    }

    /// The tree with `a/.cairn-trash/e1/old.txt` reserved as a trash folder, `a` standing for the server root.
    fn with_trash(t: &Tree) -> Reserved {
        let a = t.dir.path().join("a");
        std::fs::create_dir_all(a.join(".cairn-trash/e1")).unwrap();
        std::fs::write(a.join(".cairn-trash/e1/old.txt"), "old").unwrap();
        symlink(".cairn-trash", a.join("trash-link")).unwrap();
        Reserved {
            data: Some(t.data_id),
            trash: vec![id_of_path(&a.join(".cairn-trash"))].into(),
            root: Some(id_of_path(&a)),
        }
    }

    #[test]
    fn trash_folder_is_not_found() {
        let t = tree();
        let reserved = with_trash(&t);
        let [modern, old] = scopes_with(&t, reserved);
        for s in [&modern, &old] {
            assert!(matches!(
                s.open_dir(&p(".cairn-trash")),
                Err(FsError::NotFound)
            ));
            assert!(matches!(
                read(s, ".cairn-trash/e1/old.txt"),
                Err(FsError::NotFound)
            ));
            assert!(matches!(
                s.parent(&p(".cairn-trash")),
                Err(FsError::NotFound)
            ));
            assert!(matches!(s.stat(&p(".cairn-trash")), Err(FsError::NotFound)));
            let names: Vec<String> = crate::fs::list::list(s, &p(""))
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
            assert!(!names.contains(&".cairn-trash".to_string()), "{names:?}");
        }
        assert!(matches!(
            modern.open_dir(&p("trash-link")),
            Err(FsError::NotFound)
        ));
        assert!(matches!(
            read(&modern, "trash-link/e1/old.txt"),
            Err(FsError::NotFound)
        ));
    }

    #[test]
    fn trash_name_refused_at_root_only() {
        let t = tree();
        let a = t.dir.path().join("a");
        std::fs::create_dir_all(a.join("docs")).unwrap();
        std::fs::create_dir_all(a.join("photos")).unwrap();
        let reserved = Reserved {
            data: Some(t.data_id),
            trash: Vec::new().into(),
            root: Some(id_of_path(&a)),
        };
        // `photos` is its own mount; `docs` is not.
        crate::fs::mount::fake_mount_id(&a, 7001);
        crate::fs::mount::fake_mount_id(&a.join("docs"), 7001);
        crate::fs::mount::fake_mount_id(&a.join("photos"), 7002);
        crate::fs::mount::fake_mount_id(t.dir.path(), 7000);
        for s in scopes_with(&t, reserved) {
            assert!(matches!(
                crate::fs::edit::mkdir(&s, &p(".cairn-trash")),
                Err(FsError::Reserved)
            ));
            assert!(matches!(
                crate::fs::edit::mkdir(&s, &p("photos/.cairn-trash")),
                Err(FsError::Reserved)
            ));
            assert!(matches!(
                crate::fs::ops::apply(
                    &s,
                    crate::fs::ops::Op::Rename,
                    &p("f.txt"),
                    &p(".cairn-trash")
                ),
                Err(FsError::Reserved)
            ));
            crate::fs::edit::mkdir(&s, &p("docs/.cairn-trash")).unwrap();
            rustix::fs::unlinkat(
                s.open_dir(&p("docs")).unwrap(),
                ".cairn-trash",
                AtFlags::REMOVEDIR,
            )
            .unwrap();
        }
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

    #[test]
    fn sub_is_a_scope() {
        let t = tree();
        for s in scopes(&t) {
            let sub = s.sub(&p("sub")).unwrap();
            assert_eq!(read(&sub, "g.txt").unwrap(), "g");
            assert!(matches!(s.sub(&p("f.txt")), Err(FsError::NotAFolder)));
        }
    }

    #[test]
    fn sub_confines_links() {
        let t = tree();
        // a/sub/back -> ../f.txt: inside the scope, outside the sub-scope.
        symlink("../f.txt", t.dir.path().join("a/sub/back")).unwrap();
        for s in scopes(&t) {
            assert_eq!(
                read(&s, "sub/back").is_ok(),
                matches!(s.mode, Resolve::Openat2)
            );
            let sub = s.sub(&p("sub")).unwrap();
            assert!(matches!(read(&sub, "back"), Err(FsError::Outside)));
        }
    }
}
