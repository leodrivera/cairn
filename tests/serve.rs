//! Runs the real binary: startup, health, shutdown.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_cairn");

struct Folders {
    root: tempfile::TempDir,
    data: tempfile::TempDir,
}

fn folders() -> Folders {
    Folders {
        root: tempfile::tempdir().unwrap(),
        data: tempfile::tempdir().unwrap(),
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn cairn(f: &Folders, port: u16, args: &[&str]) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .env_clear()
        .env("CAIRN_ROOT", f.root.path())
        .env("CAIRN_DATA", f.data.path())
        .env("CAIRN_ADDR", format!("127.0.0.1:{port}"));
    cmd
}

/// Status code of GET /api/health, or None when nothing answers.
fn health_status(port: u16) -> Option<u16> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream
        .write_all(b"GET /api/health HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .ok()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).ok()?;
    reply.split(' ').nth(1)?.parse().ok()
}

fn wait_healthy(port: u16) {
    let start = Instant::now();
    while health_status(port) != Some(200) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "server did not answer within 5 s"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn sigterm(child: &Child) {
    let ok = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap()
        .success();
    assert!(ok);
}

#[test]
fn invalid_config_exits_2() {
    let f = folders();
    let out = cairn(&f, free_port(), &[])
        .env("CAIRN_LOG", "loud")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("CAIRN_LOG:"));
}

#[test]
fn missing_root_exits_2() {
    let f = folders();
    let out = cairn(&f, free_port(), &["serve"])
        .env("CAIRN_ROOT", "/x/missing")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("CAIRN_ROOT: /x/missing does not exist.")
    );
}

#[test]
fn serves_health_and_stops_on_sigterm() {
    let f = folders();
    let port = free_port();
    let mut child = cairn(&f, port, &["serve"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    wait_healthy(port);

    sigterm(&child);
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "still running 1 s after SIGTERM"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(0));

    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    for event in ["startup.listening", "startup.captcha_disabled", "shutdown"] {
        assert!(
            stdout.contains(&format!(r#""event":"{event}""#)),
            "{event} missing from:\n{stdout}"
        );
    }
    assert!(
        stdout.lines().all(|l| l.starts_with('{')),
        "not JSON lines:\n{stdout}"
    );
}
