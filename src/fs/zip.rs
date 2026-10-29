//! Folders as ZIP: store only (no compression, so the low-end PC does not spend CPU), streamed as it is written.

use std::ffi::OsStr;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;

use axum::body::Bytes;
use rustix::fs::{AtFlags, FileType, OFlags};
use tokio::sync::mpsc::Sender;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::FsError;
use super::path::RelPath;
use super::scope::Scope;

type Chunk = Result<Bytes, std::io::Error>;

/// Hands what the ZIP writer produces to the response body, 64 KiB at a time.
struct Channel {
    tx: Sender<Chunk>,
    buf: Vec<u8>,
}

impl Channel {
    fn send(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::from(std::mem::take(&mut self.buf));
        self.tx
            .blocking_send(Ok(chunk))
            .map_err(|_| std::io::Error::other("the client went away"))
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

/// Streams the folder `path` as a ZIP. The folder was checked before the response started; a later error ends the
/// body with an error, so the browser shows a failed download instead of a short archive.
pub fn stream_zip(scope: Scope, path: RelPath) -> axum::body::Body {
    let (tx, rx) = tokio::sync::mpsc::channel::<Chunk>(8);
    tokio::task::spawn_blocking(move || {
        let failed = tx.clone();
        let mut zip = ZipWriter::new_stream(Channel {
            tx,
            buf: Vec::new(),
        });
        let result = add_folder(&mut zip, &scope, &path, "").and_then(|()| {
            let mut out = zip.finish().map_err(zip_err)?;
            out.flush().map_err(FsError::from)
        });
        if let Err(e) = result {
            tracing::warn!(event = "file.zip_failed", path = %path, error = ?e);
            let _ = failed.blocking_send(Err(std::io::Error::other(
                "the archive could not be completed",
            )));
        }
    });
    axum::body::Body::from_stream(tokio_stream(rx))
}

fn tokio_stream(
    mut rx: tokio::sync::mpsc::Receiver<Chunk>,
) -> impl futures_util::Stream<Item = Chunk> {
    futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx))
}

fn add_folder<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    scope: &Scope,
    rel: &RelPath,
    prefix: &str,
) -> Result<(), FsError> {
    let dir = scope.open_dir(rel)?;
    let mut names: Vec<Vec<u8>> = Vec::new();
    for item in rustix::fs::Dir::read_from(&dir)? {
        let raw = item?.file_name().to_bytes().to_vec();
        if raw != b"." && raw != b".." && !raw.starts_with(super::list::TEMP_PREFIX.as_bytes()) {
            names.push(raw);
        }
    }
    names.sort();
    for raw in names {
        let Ok(child) = rel.join_bytes(&raw) else {
            continue;
        };
        let st = rustix::fs::statat(&dir, OsStr::from_bytes(&raw), AtFlags::SYMLINK_NOFOLLOW)?;
        if scope.data_id() == Some((st.st_dev, st.st_ino)) {
            continue;
        }
        let entry = format!("{prefix}{}", String::from_utf8_lossy(&raw));
        match FileType::from_raw_mode(st.st_mode) {
            FileType::Directory => {
                zip.add_directory(format!("{entry}/"), options())
                    .map_err(zip_err)?;
                add_folder(zip, scope, &child, &format!("{entry}/"))?;
            }
            FileType::RegularFile => add_file(zip, scope, &child, &entry)?,
            // A link is followed only to a file inside the scope; links to folders or outside are left out.
            FileType::Symlink => {
                if let Ok(fd) = scope.open(&child, OFlags::PATH)
                    && FileType::from_raw_mode(rustix::fs::fstat(&fd)?.st_mode)
                        == FileType::RegularFile
                {
                    add_file(zip, scope, &child, &entry)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn add_file<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    scope: &Scope,
    rel: &RelPath,
    entry: &str,
) -> Result<(), FsError> {
    let fd = scope.open(rel, OFlags::RDONLY)?;
    zip.start_file(entry, options()).map_err(zip_err)?;
    std::io::copy(&mut std::fs::File::from(fd), zip)?;
    Ok(())
}
