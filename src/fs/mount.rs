//! Which mount a folder is on, by the kernel's mount id (`statx` with `STATX_MNT_ID`, Linux 5.8). Two bind mounts
//! of one disk share a device number but not a mount id, and `rename` refuses to cross either.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, StatxFlags};

/// Tests cannot mount; they name folders that stand for mount points. Keyed by real path, which a temporary folder
/// never shares with another test (an inode number can be reused once a folder is gone).
fn fakes() -> &'static Mutex<HashMap<PathBuf, u64>> {
    static FAKES: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();
    FAKES.get_or_init(Default::default)
}

/// Tests only: the folder at `path` reports `mnt` as its mount id.
#[doc(hidden)]
pub fn fake_mount_id(path: &Path, mnt: u64) {
    let real = std::fs::canonicalize(path).expect("faked folder exists");
    fakes().lock().expect("fakes").insert(real, mnt);
}

/// The mount id of `fd`, or `None` where the kernel cannot tell.
pub fn mount_id(fd: &OwnedFd) -> Option<u64> {
    {
        let fakes = fakes().lock().expect("fakes");
        if !fakes.is_empty()
            && let Ok(real) = std::fs::read_link(format!(
                "/proc/self/fd/{}",
                rustix::fd::AsRawFd::as_raw_fd(fd)
            ))
            && let Some(m) = fakes.get(&real)
        {
            return Some(*m);
        }
    }
    let x = rustix::fs::statx(fd, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID).ok()?;
    StatxFlags::from_bits_retain(x.stx_mask)
        .contains(StatxFlags::MNT_ID)
        .then_some(x.stx_mnt_id)
}
