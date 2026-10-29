//! Runs `cairn user` and `cairn backup` against a temporary data folder.

use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_cairn");

struct Env {
    root: tempfile::TempDir,
    data: tempfile::TempDir,
    extra: Vec<(String, String)>,
}

fn env() -> Env {
    Env {
        root: tempfile::tempdir().unwrap(),
        data: tempfile::tempdir().unwrap(),
        extra: Vec::new(),
    }
}

impl Env {
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(BIN);
        c.args(args)
            .env_clear()
            .env("CAIRN_ROOT", self.root.path())
            .env("CAIRN_DATA", self.data.path());
        for (k, v) in &self.extra {
            c.env(k, v);
        }
        c
    }

    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = self
            .cmd(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

const ADMIN: &[&str] = &[
    "user",
    "add",
    "Admin",
    "admin@example.com",
    "--admin",
    "--password",
];

#[test]
fn add_admin_with_password() {
    let e = env();
    let out = e.run(ADMIN, "Abcdef1!\n");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout).trim(),
        "Created admin@example.com (admin, scope /)."
    );
    let list = e.run(&["user", "list"], "");
    assert_eq!(
        text(&list.stdout).trim(),
        "admin@example.com\tAdmin\t/\trwsa\tpassword"
    );
}

#[test]
fn weak_password_refused() {
    let e = env();
    let out = e.run(ADMIN, "short\n");
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("The password needs at least 8 characters"));
}

#[test]
fn password_required_without_sso() {
    let e = env();
    let out = e.run(&["user", "add", "Guest", "guest@example.com"], "");
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains(
        "This server signs in with passwords only, so the user needs one. Add --password."
    ));
}

#[test]
fn oauth_only_user_allowed() {
    let mut e = env();
    for (k, v) in [
        ("CAIRN_AUTH_METHODS", "password,oidc"),
        ("CAIRN_PUBLIC_URL", "https://files.example.com"),
        ("CAIRN_OIDC_PROVIDERS", "google"),
        ("CAIRN_OIDC_GOOGLE_ISSUER", "https://accounts.google.com"),
        ("CAIRN_OIDC_GOOGLE_CLIENT_ID", "id"),
        ("CAIRN_OIDC_GOOGLE_CLIENT_SECRET", "s"),
    ] {
        e.extra.push((k.into(), v.into()));
    }
    let out = e.run(
        &[
            "user",
            "add",
            "Guest",
            "guest@example.com",
            "--scope",
            "/public",
        ],
        "",
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let list = e.run(&["user", "list"], "");
    assert_eq!(
        text(&list.stdout).trim(),
        "guest@example.com\tGuest\t/public\tr---\tno password"
    );
}

#[test]
fn passwd_changes_it() {
    let e = env();
    e.run(ADMIN, "Abcdef1!\n");
    let out = e.run(&["user", "passwd", "admin@example.com"], "Ghijkl2?\n");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout).trim(),
        "Password changed for admin@example.com. Their sessions have ended."
    );
}

#[test]
fn cannot_delete_last_admin() {
    let e = env();
    e.run(ADMIN, "Abcdef1!\n");
    let out = e.run(&["user", "delete", "admin@example.com"], "");
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("Cairn needs at least one admin."));
}

#[test]
fn backup_writes_copy() {
    let e = env();
    e.run(ADMIN, "Abcdef1!\n");
    let dest = e.data.path().join("backup.db");
    let out = e.run(&["backup", dest.to_str().unwrap()], "");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(std::fs::metadata(&dest).unwrap().len() > 0);
}

#[test]
fn serve_logs_no_users() {
    let e = env();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut child = e
        .cmd(&["serve"])
        .env("CAIRN_ADDR", format!("127.0.0.1:{port}"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "server did not start"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    child.wait().unwrap();
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    assert!(out.contains(r#""event":"startup.no_users""#), "{out}");
}
