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

fn fake_points() -> &'static Mutex<Vec<PathBuf>> {
    static POINTS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    POINTS.get_or_init(Default::default)
}

/// Tests only: the folder at `path` is listed as a mount point.
#[doc(hidden)]
pub fn fake_mount_point(path: &Path) {
    let real = std::fs::canonicalize(path).expect("faked folder exists");
    fake_points().lock().expect("fake points").push(real);
}

/// Refuses `name` in `dir` when it is a mount point or holds one: moving it into the trash would carry the other
/// disk along, and removing it would empty that disk.
pub fn refuse_mounts_inside(dir: &OwnedFd, name: &std::ffi::OsStr) -> Result<(), super::FsError> {
    let Ok(here) = std::fs::read_link(format!(
        "/proc/self/fd/{}",
        rustix::fd::AsRawFd::as_raw_fd(dir)
    )) else {
        return Ok(());
    };
    let item = here.join(name);
    if mount_points(&here).iter().any(|p| p.starts_with(&item)) {
        return Err(super::FsError::HoldsMount(
            name.to_string_lossy().into_owned(),
        ));
    }
    Ok(())
}

/// Mount points at or below `root`, from `/proc/self/mountinfo`.
pub fn mount_points(root: &Path) -> Vec<PathBuf> {
    let fakes: Vec<PathBuf> = fake_points()
        .lock()
        .expect("fake points")
        .iter()
        .filter(|p| *p != root && p.starts_with(root))
        .cloned()
        .collect();
    let Ok(text) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return fakes;
    };
    let unescape = |s: &str| {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\\'
                && i + 3 < b.len()
                && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
            {
                out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
                i += 4;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(out))
    };
    text.lines()
        .filter_map(|l| l.split(' ').nth(4).map(unescape))
        .filter(|p| p != root && p.starts_with(root))
        .chain(fakes)
        .collect()
}
