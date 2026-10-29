//! Secrets from the environment, or from a file named by `<NAME>_FILE` (Docker and Compose secrets).

use std::fmt;

use super::Vars;

/// A value that must never reach a log line or a panic message.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

/// Reads `name`, or the file named by `<name>_FILE`, dropping one trailing newline from the file.
pub fn read_secret(vars: &Vars, name: &str) -> Result<Option<Secret>, String> {
    let file_var = format!("{name}_FILE");
    match (vars.get(name), vars.get(&file_var)) {
        (Some(_), Some(_)) => Err(format!(
            "{name}: set it or {file_var}, not both. Remove one of them."
        )),
        (Some(value), None) => Ok(Some(Secret(value.clone())).filter(|s| !s.0.is_empty())),
        (None, Some(path)) => {
            let text = std::fs::read_to_string(path).map_err(|e| {
                format!("{file_var}: cannot read {path} ({e}). Check the path and that the user running Cairn can read it.")
            })?;
            let text = text
                .strip_suffix("\r\n")
                .or_else(|| text.strip_suffix('\n'))
                .unwrap_or(&text);
            Ok(Some(Secret(text.to_string())).filter(|s| !s.0.is_empty()))
        }
        (None, None) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vars {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn file_with(content: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file
    }

    #[test]
    fn plain_value() {
        let got = read_secret(
            &vars(&[("CAIRN_OAUTH_SECRET", "abc")]),
            "CAIRN_OAUTH_SECRET",
        )
        .unwrap();
        assert_eq!(got.unwrap().expose(), "abc");
    }

    #[test]
    fn absent_is_none() {
        assert!(
            read_secret(&Vars::new(), "CAIRN_OAUTH_SECRET")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn file_drops_one_trailing_newline() {
        for (content, expected) in [
            ("s3cret\n", "s3cret"),
            ("s3cret\r\n", "s3cret"),
            ("s3cret\n\n", "s3cret\n"),
            ("s3cret", "s3cret"),
        ] {
            let file = file_with(content);
            let path = file.path().to_str().unwrap();
            let got = read_secret(
                &vars(&[("CAIRN_OAUTH_SECRET_FILE", path)]),
                "CAIRN_OAUTH_SECRET",
            )
            .unwrap();
            assert_eq!(got.unwrap().expose(), expected, "file content {content:?}");
        }
    }

    #[test]
    fn both_set_is_error() {
        let file = file_with("x");
        let path = file.path().to_str().unwrap();
        let got = read_secret(
            &vars(&[
                ("CAIRN_OAUTH_SECRET", "x"),
                ("CAIRN_OAUTH_SECRET_FILE", path),
            ]),
            "CAIRN_OAUTH_SECRET",
        );
        assert_eq!(got, Err("CAIRN_OAUTH_SECRET: set it or CAIRN_OAUTH_SECRET_FILE, not both. Remove one of them.".into()));
    }

    #[test]
    fn unreadable_file_is_error() {
        let got = read_secret(
            &vars(&[("CAIRN_OAUTH_SECRET_FILE", "/nonexistent")]),
            "CAIRN_OAUTH_SECRET",
        )
        .unwrap_err();
        assert!(
            got.starts_with("CAIRN_OAUTH_SECRET_FILE: cannot read /nonexistent ("),
            "{got}"
        );
    }

    #[test]
    fn empty_is_unset() {
        let file = file_with("\n");
        let path = file.path().to_str().unwrap();
        assert!(
            read_secret(
                &vars(&[("CAIRN_OAUTH_SECRET_FILE", path)]),
                "CAIRN_OAUTH_SECRET"
            )
            .unwrap()
            .is_none()
        );
        assert!(
            read_secret(&vars(&[("CAIRN_OAUTH_SECRET", "")]), "CAIRN_OAUTH_SECRET")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn debug_is_redacted() {
        assert_eq!(
            format!("{:?}", Secret("hunter2".into())),
            "Secret([redacted])"
        );
    }
}
