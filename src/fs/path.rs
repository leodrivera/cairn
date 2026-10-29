//! A path from a client: relative to the user's scope, checked segment by segment.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use super::FsError;

/// Segments are raw bytes: Linux names need not be UTF-8. On the wire a segment that is not UTF-8 is written as a
/// NUL followed by its bytes in base64url (see `segment_for`). Real names cannot contain NUL, so this never
/// collides with one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelPath(Vec<Vec<u8>>);

fn valid_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name != b"."
        && name != b".."
        && name.len() <= 255
        && !name.contains(&b'/')
        && !name.contains(&0)
}

fn decode_segment(seg: &str) -> Option<Vec<u8>> {
    match seg.strip_prefix('\0') {
        Some(encoded) => URL_SAFE_NO_PAD.decode(encoded).ok(),
        None => Some(seg.as_bytes().to_vec()),
    }
}

impl RelPath {
    /// One leading and one trailing `/` are ignored; the empty path is the scope itself.
    pub fn parse(raw: &str) -> Result<RelPath, FsError> {
        if raw.len() > 4096 {
            return Err(FsError::InvalidPath);
        }
        let trimmed = raw.strip_prefix('/').unwrap_or(raw);
        let trimmed = trimmed.strip_suffix('/').unwrap_or(trimmed);
        if trimmed.is_empty() {
            return Ok(RelPath(Vec::new()));
        }
        let mut parts = Vec::new();
        for seg in trimmed.split('/') {
            match decode_segment(seg) {
                Some(bytes) if valid_name(&bytes) => parts.push(bytes),
                _ => return Err(FsError::InvalidPath),
            }
        }
        Ok(RelPath(parts))
    }

    /// How a name is written in a path: as itself when it is UTF-8, else NUL plus base64url of its bytes.
    pub fn segment_for(name: &[u8]) -> String {
        match std::str::from_utf8(name) {
            Ok(text) => text.to_string(),
            Err(_) => format!("\0{}", URL_SAFE_NO_PAD.encode(name)),
        }
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn parent(&self) -> Option<RelPath> {
        let (_, rest) = self.0.split_last()?;
        Some(RelPath(rest.to_vec()))
    }

    /// The final name, when it is UTF-8.
    pub fn name(&self) -> Option<&str> {
        self.0.last().and_then(|n| std::str::from_utf8(n).ok())
    }

    pub fn name_bytes(&self) -> Option<&[u8]> {
        self.0.last().map(Vec::as_slice)
    }

    /// The final name for people: unreadable bytes become U+FFFD.
    pub fn name_lossy(&self) -> Option<Cow<'_, str>> {
        self.0.last().map(|n| String::from_utf8_lossy(n))
    }

    pub fn join(&self, name: &str) -> Result<RelPath, FsError> {
        self.join_bytes(name.as_bytes())
    }

    pub fn join_bytes(&self, name: &[u8]) -> Result<RelPath, FsError> {
        if !valid_name(name) {
            return Err(FsError::InvalidPath);
        }
        let mut parts = self.0.clone();
        parts.push(name.to_vec());
        Ok(RelPath(parts))
    }

    /// By whole segments: `a/bc` does not start with `a/b`.
    pub fn starts_with(&self, other: &RelPath) -> bool {
        self.0.starts_with(&other.0)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The first `n` segments.
    pub fn prefix(&self, n: usize) -> RelPath {
        RelPath(self.0[..n.min(self.0.len())].to_vec())
    }

    /// Relative, for `openat2`; `.` for the root.
    pub fn to_os(&self) -> PathBuf {
        if self.0.is_empty() {
            return PathBuf::from(".");
        }
        PathBuf::from(OsStr::from_bytes(&self.0.join(&b'/')))
    }
}

/// The wire form: what a client sends back to reach the same path.
impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let segments: Vec<String> = self.0.iter().map(|s| RelPath::segment_for(s)).collect();
        f.write_str(&segments.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    fn p(s: &str) -> RelPath {
        RelPath::parse(s).unwrap()
    }

    #[test]
    fn parses_segments() {
        assert_eq!(p("a/b/c").to_string(), "a/b/c");
        assert_eq!(p("/a/b/"), p("a/b"));
        assert!(p("").is_root());
        assert!(p("/").is_root());
    }

    #[test]
    fn refuses_bad() {
        let long_name = "x".repeat(256);
        let long_path = format!("{}/{}", "a".repeat(2048), "b".repeat(2048));
        for bad in [
            "a/../b",
            "./a",
            "a//b",
            "a\0b",
            "..",
            long_name.as_str(),
            long_path.as_str(),
        ] {
            assert!(
                matches!(RelPath::parse(bad), Err(FsError::InvalidPath)),
                "{bad:?}"
            );
        }
        assert!(RelPath::parse(&"x".repeat(255)).is_ok());
    }

    #[test]
    fn unicode_kept() {
        assert_eq!(
            p("Report #1 (50%) ✓ café.txt").to_string(),
            "Report #1 (50%) ✓ café.txt"
        );
    }

    #[test]
    fn parent_and_name() {
        let x = p("a/b/c.txt");
        assert_eq!(x.name(), Some("c.txt"));
        assert_eq!(x.parent(), Some(p("a/b")));
        assert_eq!(p("a").parent(), Some(p("")));
        assert_eq!(p("").parent(), None);
        assert_eq!(p("").name(), None);
    }

    #[test]
    fn join_refuses_slash() {
        assert_eq!(p("a").join("b").unwrap(), p("a/b"));
        assert!(matches!(p("a").join("b/c"), Err(FsError::InvalidPath)));
        assert!(matches!(p("a").join(".."), Err(FsError::InvalidPath)));
    }

    #[test]
    fn starts_with_by_segment() {
        assert!(p("a/b/c").starts_with(&p("a/b")));
        assert!(p("a/b").starts_with(&p("a/b")));
        assert!(!p("a/bc").starts_with(&p("a/b")));
        assert!(p("x").starts_with(&p("")));
        assert_eq!(p("a/b").to_os(), std::path::PathBuf::from("a/b"));
    }

    #[test]
    fn encoded_segment_carries_raw_bytes() {
        use base64::Engine;
        let raw = b"Relat\xf3rio.txt";
        let seg = format!(
            "\0{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
        );
        let path = RelPath::parse(&format!("docs/{seg}")).unwrap();
        assert_eq!(path.name_bytes(), Some(&raw[..]));
        assert_eq!(
            path.to_string(),
            format!("docs/{seg}"),
            "Display keeps the encoded form"
        );
        assert_eq!(path.name_lossy().as_deref(), Some("Relat\u{FFFD}rio.txt"));
        assert_eq!(
            path.to_os(),
            std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"docs/Relat\xf3rio.txt"))
        );
    }

    #[test]
    fn encoded_segment_rules() {
        use base64::Engine;
        let enc = |b: &[u8]| {
            format!(
                "\0{}",
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
            )
        };
        assert!(matches!(
            RelPath::parse("\0not base64!"),
            Err(FsError::InvalidPath)
        ));
        for bad in [&b"a/b"[..], b"a\0b", b"..", b"."] {
            assert!(
                matches!(RelPath::parse(&enc(bad)), Err(FsError::InvalidPath)),
                "{bad:?}"
            );
        }
        // Valid UTF-8 sent encoded is the same path as sent plain.
        assert_eq!(RelPath::parse(&enc(b"plain.txt")).unwrap(), p("plain.txt"));
        assert_eq!(RelPath::segment_for(b"plain.txt"), "plain.txt");
        assert_eq!(RelPath::segment_for(b"Relat\xf3rio"), enc(b"Relat\xf3rio"));
    }
}
