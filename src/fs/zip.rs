//! Folders as ZIP: store only (no compression, so the low-end PC does not spend CPU), streamed as it is written.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::Write;
use std::net::IpAddr;
use std::os::unix::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use rustix::fs::{AtFlags, FileType, OFlags};
use tokio::sync::mpsc::Sender;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::path::RelPath;
use super::scope::Scope;
use super::{FsError, MAX_DEPTH};

/// Archives running at once, each on a thread of the blocking pool that every file and sign-in also needs.
pub const MAX_ZIPS: usize = 16;
pub const MAX_ZIPS_PER_CLIENT: usize = 4;
/// How long a client may leave the archive unread before it ends with an error.
pub const SEND_WAIT: Duration = Duration::from_secs(60);

type Chunk = Result<Bytes, std::io::Error>;

#[derive(Default)]
struct Running {
    total: usize,
    clients: HashMap<String, usize>,
}

/// Counts the archives running on this server.
#[derive(Default)]
pub struct ZipSlots(Mutex<Running>);

/// One running archive. Dropping it gives the place back.
pub struct ZipSlot {
    slots: Arc<ZipSlots>,
    client: String,
}

impl ZipSlots {
    /// A place for one more archive for `ip`, or `None` over a limit.
    pub fn take(self: &Arc<Self>, ip: IpAddr) -> Option<ZipSlot> {
        let client = crate::auth::ratelimit::ip_key(ip);
        let mut running = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let mine = running.clients.get(&client).copied().unwrap_or(0);
        if running.total >= MAX_ZIPS || mine >= MAX_ZIPS_PER_CLIENT {
            return None;
        }
        running.total += 1;
        running.clients.insert(client.clone(), mine + 1);
        Some(ZipSlot {
            slots: self.clone(),
            client,
        })
    }
}

impl Drop for ZipSlot {
    fn drop(&mut self) {
        let mut running = self.slots.0.lock().unwrap_or_else(|p| p.into_inner());
        running.total -= 1;
        if let Some(mine) = running.clients.get_mut(&self.client) {
            *mine -= 1;
            if *mine == 0 {
                running.clients.remove(&self.client);
            }
        }
    }
}

/// Hands what the ZIP writer produces to the response body, 64 KiB at a time.
struct Channel {
    tx: Sender<Bytes>,
    buf: Vec<u8>,
    runtime: tokio::runtime::Handle,
    wait: Duration,
    gone: bool,
}

impl Channel {
    /// Waits for the client, but never past `wait`: this holds a thread of the blocking pool. Once a send has failed
    /// every later one fails at once, so closing the archive does not wait again.
    fn send(&mut self) -> std::io::Result<()> {
        let went_away = || std::io::Error::other("the client went away");
        if self.gone {
            return Err(went_away());
        }
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::from(std::mem::take(&mut self.buf));
        let sent = self
            .runtime
            .block_on(tokio::time::timeout(self.wait, self.tx.send(chunk)));
        if !matches!(sent, Ok(Ok(()))) {
            self.gone = true;
            return Err(went_away());
        }
        Ok(())
    }
}

impl Write for Channel {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= 64 * 1024 {
            self.send()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.send()
    }
}

fn options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .large_file(true)
}

fn zip_err(e: zip::result::ZipError) -> FsError {
    FsError::Io(std::io::Error::other(e))
}

/// Streams the folder `path` as a ZIP, or only the chosen `items` inside it, each under its own name. The folder and
/// items were checked before the response started; a later error, or a client that reads nothing for `wait`, ends the
/// body with an error, so the browser shows a failed download instead of a short archive. `slot` lasts as long as the
/// archive does.
pub fn stream_zip(
    scope: Scope,
    path: RelPath,
    items: Option<Vec<RelPath>>,
    slot: ZipSlot,
    wait: Duration,
) -> axum::body::Body {
    let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(8);
    let complete = Arc::new(AtomicBool::new(false));
    let finished = complete.clone();
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        let mut zip = ZipWriter::new_stream(Channel {
            tx,
            buf: Vec::new(),
            runtime,
            wait,
            gone: false,
        });
        let added = match &items {
            None => add_folder(&mut zip, &scope, &path, "", 0),
            Some(items) => {
                let mut used = HashSet::new();
                items.iter().try_for_each(|item| {
                    let st = scope.stat(item)?;
                    let raw = item.name_bytes().unwrap_or_default();
                    let entry = entry_name(raw, &st, &mut used);
                    add_child(&mut zip, &scope, item, &st, &entry, 0)
                })
            }
        };
        let result = added.and_then(|()| {
            let mut out = zip.finish().map_err(zip_err)?;
            out.flush().map_err(FsError::from)?;
            // Before `out` closes the channel, so the stream never sees the end without this.
            finished.store(true, Ordering::Release);
            Ok(())
        });
        if let Err(e) = result {
            tracing::warn!(event = "file.zip_failed", path = %path, error = ?e);
        }
    });
    axum::body::Body::from_stream(chunks(rx, complete))
}

/// The archive's bytes, then an error unless the writer finished: a channel closed early must not look like the end.
fn chunks(
    mut rx: tokio::sync::mpsc::Receiver<Bytes>,
    complete: Arc<AtomicBool>,
) -> impl futures_util::Stream<Item = Chunk> {
    let mut ended = false;
    futures_util::stream::poll_fn(move |cx| {
        if ended {
            return std::task::Poll::Ready(None);
        }
        rx.poll_recv(cx).map(|chunk| match chunk {
            Some(bytes) => Some(Ok(bytes)),
            None => {
                ended = true;
                (!complete.load(Ordering::Acquire))
                    .then(|| Err(std::io::Error::other("the archive could not be completed")))
            }
        })
    })
}

/// The name of an entry in its folder. Names on disk are bytes and may hold what a ZIP path should not, and two can read
/// the same once made readable, which a ZIP refuses. Each one gets a readable, unique name; `used` is that folder's.
fn entry_name(raw: &[u8], st: &rustix::fs::Stat, used: &mut HashSet<String>) -> String {
    let name: String = String::from_utf8_lossy(raw)
        .chars()
        .map(|c| if c == '\\' || c.is_control() { '_' } else { c })
        .collect();
    if used.insert(name.clone()) {
        return name;
    }
    let folder = FileType::from_raw_mode(st.st_mode) == FileType::Directory;
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > 0 && !folder => name.split_at(dot),
        _ => (name.as_str(), ""),
    };
    (2..)
        .map(|n| format!("{stem} ({n}){extension}"))
        .find(|candidate| used.insert(candidate.clone()))
        .expect("the numbers do not run out")
}

fn add_folder<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    scope: &Scope,
    rel: &RelPath,
    prefix: &str,
    depth: usize,
) -> Result<(), FsError> {
    if depth > MAX_DEPTH {
        return Err(FsError::TooDeep);
    }
    let dir = scope.open_dir(rel)?;
    let mut names: Vec<Vec<u8>> = Vec::new();
    for item in rustix::fs::Dir::read_from(&dir)? {
        let raw = item?.file_name().to_bytes().to_vec();
        if raw != b"." && raw != b".." && !raw.starts_with(super::list::TEMP_PREFIX.as_bytes()) {
            names.push(raw);
        }
    }
    names.sort();
    let mut used = HashSet::new();
    for raw in names {
        let Ok(child) = rel.join_bytes(&raw) else {
            continue;
        };
        let st = rustix::fs::statat(&dir, OsStr::from_bytes(&raw), AtFlags::SYMLINK_NOFOLLOW)?;
        if scope.is_reserved((st.st_dev, st.st_ino)) {
            continue;
        }
        let entry = format!("{prefix}{}", entry_name(&raw, &st, &mut used));
        add_child(zip, scope, &child, &st, &entry, depth)?;
    }
    Ok(())
}

/// One entry of a folder: a folder recursively, a file, or a link to a file inside the scope.
fn add_child<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    scope: &Scope,
    child: &RelPath,
    st: &rustix::fs::Stat,
    entry: &str,
    depth: usize,
) -> Result<(), FsError> {
    match FileType::from_raw_mode(st.st_mode) {
        FileType::Directory => {
            zip.add_directory(format!("{entry}/"), options())
                .map_err(zip_err)?;
            add_folder(zip, scope, child, &format!("{entry}/"), depth + 1)?;
        }
        FileType::RegularFile => add_file(zip, scope, child, entry)?,
        // A link is followed only to a file inside the scope; links to folders or outside are left out.
        FileType::Symlink => {
            if let Ok(fd) = scope.open(child, OFlags::PATH)
                && FileType::from_raw_mode(rustix::fs::fstat(&fd)?.st_mode) == FileType::RegularFile
            {
                add_file(zip, scope, child, entry)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn add_file<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    scope: &Scope,
    rel: &RelPath,
    entry: &str,
) -> Result<(), FsError> {
    // O_NONBLOCK and the check: the name may no longer be the file `add_folder` saw, and opening a pipe would wait.
    let fd = scope.open(rel, OFlags::RDONLY | OFlags::NONBLOCK)?;
    if FileType::from_raw_mode(rustix::fs::fstat(&fd)?.st_mode) != FileType::RegularFile {
        return Ok(());
    }
    zip.start_file(entry, options()).map_err(zip_err)?;
    std::io::copy(&mut std::fs::File::from(fd), zip)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;

    const CLIENT: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 9));

    fn client(n: u8) -> IpAddr {
        IpAddr::V4(std::net::Ipv4Addr::new(198, 51, 100, n))
    }

    #[test]
    fn one_client_has_its_own_limit() {
        let slots = Arc::new(ZipSlots::default());
        let mut held: Vec<_> = (0..MAX_ZIPS_PER_CLIENT)
            .map(|_| slots.take(CLIENT).unwrap())
            .collect();
        assert!(slots.take(CLIENT).is_none());
        assert!(slots.take(client(1)).is_some());
        held.pop();
        assert!(slots.take(CLIENT).is_some());
    }

    #[test]
    fn the_server_has_a_limit_for_everyone() {
        let slots = Arc::new(ZipSlots::default());
        let held: Vec<_> = (0..MAX_ZIPS as u8)
            .map(|n| slots.take(client(n)).unwrap())
            .collect();
        assert!(slots.take(CLIENT).is_none());
        drop(held);
        assert!(slots.take(CLIENT).is_some());
    }

    #[test]
    fn a_finished_client_leaves_no_entry() {
        let slots = Arc::new(ZipSlots::default());
        drop(slots.take(CLIENT).unwrap());
        assert!(slots.0.lock().unwrap().clients.is_empty());
    }

    /// `add_folder` checks the type and `add_file` opens later; the name can change in between.
    #[test]
    fn a_pipe_is_left_out_without_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let pipe = dir.path().join("pipe");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &pipe,
            FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o644),
            0,
        )
        .unwrap();
        let scope = Scope::new(dir.path(), "/", Default::default()).unwrap();
        let (done, answer) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut zip = ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let added = add_file(&mut zip, &scope, &RelPath::parse("pipe").unwrap(), "pipe");
            let bytes = zip.finish().unwrap().into_inner();
            let _ = done.send((added.is_ok(), bytes.windows(4).any(|w| w == b"pipe")));
        });
        let result = answer.recv_timeout(Duration::from_secs(5));
        // Opened by a writer, a stuck open lets go, so a failure here leaves no thread behind.
        let _writer = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe)
            .unwrap();
        assert_eq!(result.expect("no answer"), (true, false));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unread_archive_ends_and_never_looks_complete() {
        let dir = tempfile::tempdir().unwrap();
        // Larger than the channel, so the writer has to wait for this reader.
        let file = std::fs::File::create(dir.path().join("big.bin")).unwrap();
        file.set_len(4 * 1024 * 1024).unwrap();
        let scope = Scope::new(dir.path(), "/", Default::default()).unwrap();
        let slots = Arc::new(ZipSlots::default());
        let slot = slots.take(CLIENT).unwrap();
        let path = RelPath::parse("").unwrap();
        let wait = Duration::from_millis(100);
        let mut stream = stream_zip(scope, path, None, slot, wait).into_data_stream();

        let freed = tokio::time::timeout(Duration::from_secs(5), async {
            while slots.0.lock().unwrap().total > 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(freed.is_ok(), "the thread must let go of a silent reader");

        let mut last = None;
        while let Some(chunk) = stream.next().await {
            last = Some(chunk);
        }
        assert!(last.unwrap().is_err(), "a cut archive must not end cleanly");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deep_tree_ends_with_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = dir.path().join("deep");
        for _ in 0..300 {
            p.push("d");
        }
        std::fs::create_dir_all(&p).unwrap();
        let scope = Scope::new(dir.path(), "/", Default::default()).unwrap();
        let slot = Arc::new(ZipSlots::default()).take(CLIENT).unwrap();
        let path = RelPath::parse("deep").unwrap();
        let mut stream = stream_zip(scope, path, None, slot, SEND_WAIT).into_data_stream();
        let mut failed = false;
        while let Some(chunk) = stream.next().await {
            if chunk.is_err() {
                failed = true;
            }
        }
        assert!(
            failed,
            "a tree past the depth limit must end the download with an error"
        );
    }
}
