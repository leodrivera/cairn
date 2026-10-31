//! Whether a share link still works: expiry, the creator's permission and scope, the target on disk.

use std::os::unix::fs::{MetadataExt, symlink};

use cairn::db::Db;
use cairn::fs::path::RelPath;
use cairn::fs::scope::Scope;
use cairn::shares::{self, Live, ShareRow, Target};
use cairn::users::{self, NewUser, Perms, UserPatch};
use rustix::fs::OFlags;

const NOW: i64 = 1_800_000_000;
const SHARER: Perms = Perms {
    read: true,
    write: false,
    share: true,
    admin: false,
};

struct Fx {
    dir: tempfile::TempDir,
    db: Db,
    user: i64,
}

fn fx() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("photos/2026")).unwrap();
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(root.join("photos/2026/trail-ridge.jpg"), "jpg").unwrap();
    std::fs::write(root.join("photos/notes.txt"), "notes").unwrap();
    let db = Db::open_in_memory();
    let user = users::create(
        &db,
        NewUser {
            email: "ana@example.com".into(),
            name: "Ana".into(),
            password_hash: None,
            scope: "/".into(),
            perms: SHARER,
        },
        0,
    )
    .unwrap()
    .id;
    Fx { dir, db, user }
}

impl Fx {
    fn share(&self, path: &str, expires_at: Option<i64>) -> String {
        let row = ShareRow {
            id: shares::new_id(),
            user_id: self.user,
            path: path.into(),
            password_hash: None,
            download: true,
            expires_at,
            created_at: NOW - 10,
            hits: 0,
        };
        shares::insert(&self.db, &row, NOW - 10).unwrap();
        row.id
    }

    fn resolve_at(&self, id: &str, now: i64, data_id: Option<(u64, u64)>) -> Option<Live> {
        shares::resolve(
            &self.db,
            self.dir.path(),
            cairn::fs::scope::Reserved::data_only(data_id),
            false,
            id,
            now,
        )
    }

    fn resolve(&self, id: &str) -> Option<Live> {
        self.resolve_at(id, NOW, None)
    }

    fn patch(&self, patch: UserPatch) {
        users::update(&self.db, self.user, patch).unwrap();
    }
}

fn read(scope: &Scope, path: &str) -> Result<String, cairn::fs::FsError> {
    let fd = scope.open(&RelPath::parse(path).unwrap(), OFlags::RDONLY)?;
    Ok(std::io::read_to_string(std::fs::File::from(fd)).unwrap())
}

#[test]
fn live_file() {
    let f = fx();
    let id = f.share("/photos/2026/trail-ridge.jpg", None);
    let live = f.resolve(&id).expect("live");
    assert_eq!(live.creator.email, "ana@example.com");
    match live.target {
        Target::File { scope, path } => {
            assert_eq!(path.to_string(), "photos/2026/trail-ridge.jpg");
            assert_eq!(read(&scope, "photos/2026/trail-ridge.jpg").unwrap(), "jpg");
        }
        Target::Folder { .. } => panic!("a file"),
    }
}

#[test]
fn live_folder() {
    let f = fx();
    let id = f.share("/photos", None);
    match f.resolve(&id).expect("live").target {
        Target::Folder { scope } => assert_eq!(read(&scope, "notes.txt").unwrap(), "notes"),
        Target::File { .. } => panic!("a folder"),
    }
}

#[test]
fn unknown_id_is_gone() {
    let f = fx();
    assert!(f.resolve("k7Qw3nTzAbCdEfGhIjKl-_").is_none());
}

#[test]
fn expired_is_gone() {
    let f = fx();
    let id = f.share("/photos", Some(NOW));
    assert!(f.resolve(&id).is_none());
    assert!(f.resolve_at(&id, NOW - 1, None).is_some());
}

#[test]
fn never_expires() {
    let f = fx();
    let id = f.share("/photos", None);
    assert!(f.resolve_at(&id, NOW + 100 * 365 * 86_400, None).is_some());
}

#[test]
fn share_perm_removed_is_gone() {
    let f = fx();
    let id = f.share("/photos", None);
    f.patch(UserPatch {
        perms: Some(Perms {
            share: false,
            ..SHARER
        }),
        ..Default::default()
    });
    assert!(f.resolve(&id).is_none());
    f.patch(UserPatch {
        perms: Some(SHARER),
        ..Default::default()
    });
    assert!(f.resolve(&id).is_some());
}

#[test]
fn scope_narrowed_is_gone() {
    let f = fx();
    let id = f.share("/photos/2026/trail-ridge.jpg", None);
    f.patch(UserPatch {
        scope: Some("/docs".into()),
        ..Default::default()
    });
    assert!(f.resolve(&id).is_none());
    f.patch(UserPatch {
        scope: Some("/".into()),
        ..Default::default()
    });
    assert!(f.resolve(&id).is_some());
}

#[test]
fn scope_still_containing_is_live() {
    let f = fx();
    let id = f.share("/photos/2026/trail-ridge.jpg", None);
    f.patch(UserPatch {
        scope: Some("/photos".into()),
        ..Default::default()
    });
    match f.resolve(&id).expect("live").target {
        Target::File { scope, path } => {
            assert_eq!(path.to_string(), "2026/trail-ridge.jpg");
            assert_eq!(read(&scope, "2026/trail-ridge.jpg").unwrap(), "jpg");
        }
        Target::Folder { .. } => panic!("a file"),
    }
}

#[test]
fn target_deleted_is_gone() {
    let f = fx();
    let id = f.share("/photos/notes.txt", None);
    std::fs::remove_file(f.dir.path().join("photos/notes.txt")).unwrap();
    assert!(f.resolve(&id).is_none());
}

#[test]
fn target_moved_is_gone() {
    let f = fx();
    let id = f.share("/photos/notes.txt", None);
    std::fs::rename(
        f.dir.path().join("photos/notes.txt"),
        f.dir.path().join("docs/notes.txt"),
    )
    .unwrap();
    assert!(f.resolve(&id).is_none());
    assert!(shares::get(&f.db, &id).unwrap().is_some(), "the row stays");
    assert_eq!(
        shares::list(&f.db, None, NOW).unwrap().len(),
        1,
        "no new row"
    );
}

#[test]
fn folder_share_symlink_to_sibling_refused() {
    let f = fx();
    let root = f.dir.path();
    std::fs::create_dir_all(root.join("pub")).unwrap();
    std::fs::create_dir_all(root.join("private")).unwrap();
    std::fs::write(root.join("private/secret.txt"), "secret").unwrap();
    symlink("../private", root.join("pub/link")).unwrap();
    let id = f.share("/pub", None);
    match f.resolve(&id).expect("live").target {
        Target::Folder { scope } => assert!(read(&scope, "link/secret.txt").is_err()),
        Target::File { .. } => panic!("a folder"),
    }
}

#[test]
fn data_folder_never_shared() {
    let f = fx();
    let data = f.dir.path().join(".cairn");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("cairn.db"), "db").unwrap();
    let st = std::fs::metadata(&data).unwrap();
    let data_id = Some((st.dev(), st.ino()));
    for path in ["/.cairn", "/.cairn/cairn.db"] {
        let id = f.share(path, None);
        assert!(f.resolve_at(&id, NOW, data_id).is_none(), "{path}");
    }
}
