//! A path from a client: relative to the user's scope, checked segment by segment.

use std::fmt;
use std::path::PathBuf;

use super::FsError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelPath(Vec<String>);

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 255
        && !name.contains(['/', '\0'])
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
        let parts: Vec<String> = trimmed.split('/').map(String::from).collect();
        if parts.iter().all(|p| valid_name(p)) {
            Ok(RelPath(parts))
        } else {
            Err(FsError::InvalidPath)
        }
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn parent(&self) -> Option<RelPath> {
        let (_, rest) = self.0.split_last()?;
        Some(RelPath(rest.to_vec()))
    }

    pub fn name(&self) -> Option<&str> {
        self.0.last().map(String::as_str)
    }

    pub fn join(&self, name: &str) -> Result<RelPath, FsError> {
        if !valid_name(name) {
            return Err(FsError::InvalidPath);
        }
        let mut parts = self.0.clone();
        parts.push(name.to_string());
        Ok(RelPath(parts))
    }

    /// By whole segments: `a/bc` does not start with `a/b`.
    pub fn starts_with(&self, other: &RelPath) -> bool {
        self.0.starts_with(&other.0)
    }

    pub fn segments(&self) -> &[String] {
        &self.0
    }

    /// Relative, for `openat2`; `.` for the root.
    pub fn to_os(&self) -> PathBuf {
        if self.0.is_empty() {
            PathBuf::from(".")
        } else {
            self.0.iter().collect()
        }
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("/"))
    }
}

#[cfg(test)]
mod tests {
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
}
