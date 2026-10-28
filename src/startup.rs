//! Checks on the mounted folders, run before Cairn opens a socket.

use std::path::Path;

use crate::config::Config;

/// Every problem with `CAIRN_ROOT` and `CAIRN_DATA`; empty when both are fine.
pub fn check_paths(config: &Config) -> Vec<String> {
    let mut errors = Vec::new();
    let root = folder(
        "CAIRN_ROOT",
        &config.root,
        "Mount the folder to browse at this path.",
        &mut errors,
    );
    let data = folder(
        "CAIRN_DATA",
        &config.data,
        "Mount a folder for the database at this path.",
        &mut errors,
    );

    if let (Some(root), Some(data)) = (&root, &data)
        && data.starts_with(root)
    {
        errors.push(
            "CAIRN_DATA: is inside CAIRN_ROOT, so users could download the database. Mount it outside the server root."
                .into(),
        );
    }
    if data.is_some() {
        let probe = config.data.join(".cairn-write-test");
        match std::fs::write(&probe, b"") {
            Ok(()) => {
                let _ = std::fs::remove_file(&probe);
            }
            Err(_) => errors.push(format!(
                "CAIRN_DATA: {} is not writable by this user (uid 10001 in the image). Change the folder's owner or set user: in Compose.",
                config.data.display()
            )),
        }
    }
    errors
}

/// The canonical path of a folder that exists, or `None` after recording why not.
fn folder(
    name: &str,
    path: &Path,
    hint: &str,
    errors: &mut Vec<String>,
) -> Option<std::path::PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(real) if real.is_dir() => Some(real),
        Ok(_) => {
            errors.push(format!(
                "{name}: {} is not a folder. {hint}",
                path.display()
            ));
            None
        }
        Err(_) => {
            errors.push(format!("{name}: {} does not exist. {hint}", path.display()));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::Path;

    use super::*;
    use crate::config::{Config, Vars};

    fn config(root: &Path, data: &Path) -> Config {
        let vars: Vars = [
            ("CAIRN_ROOT".to_string(), root.display().to_string()),
            ("CAIRN_DATA".to_string(), data.display().to_string()),
        ]
        .into();
        Config::from_vars(&vars).unwrap()
    }

    const INSIDE: &str = "CAIRN_DATA: is inside CAIRN_ROOT, so users could download the database. Mount it outside the server root.";

    #[test]
    fn fine() {
        let (root, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        assert!(check_paths(&config(root.path(), data.path())).is_empty());
    }

    #[test]
    fn missing_root() {
        let data = tempfile::tempdir().unwrap();
        assert_eq!(
            check_paths(&config(Path::new("/x/missing"), data.path())),
            ["CAIRN_ROOT: /x/missing does not exist. Mount the folder to browse at this path."]
        );
    }

    #[test]
    fn data_inside_root() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("db");
        std::fs::create_dir(&data).unwrap();
        assert_eq!(check_paths(&config(root.path(), &data)), [INSIDE]);
    }

    #[test]
    fn data_inside_root_through_symlink() {
        let (root, elsewhere) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let real = root.path().join("db");
        std::fs::create_dir(&real).unwrap();
        let link = elsewhere.path().join("data");
        symlink(&real, &link).unwrap();
        assert_eq!(check_paths(&config(root.path(), &link)), [INSIDE]);
    }

    #[test]
    fn data_not_writable() {
        let (root, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::fs::set_permissions(data.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::write(data.path().join("probe"), b"").is_ok() {
            return; // running as root: permissions do not apply
        }
        let expected = format!(
            "CAIRN_DATA: {} is not writable by this user (uid 10001 in the image). Change the folder's owner or set user: in Compose.",
            data.path().display()
        );
        assert_eq!(check_paths(&config(root.path(), data.path())), [expected]);
        std::fs::set_permissions(data.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
