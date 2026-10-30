//! Share link log lines: management events, wrong link passwords and their limit. Alone in its own test binary: other tests hitting the same log callsites on other threads
//! race with the scoped subscriber this test installs.

mod common;

use cairn::users::Perms;
use common::{Req, harness_with_root};
use serde_json::json;

const PW: &str = "Abcdef1!";

#[derive(Clone, Default)]
struct LogBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn log_events() {
    let buf = LogBuf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let (h, dir) = harness_with_root(&[]);
    std::fs::write(dir.path().join("srv/other.txt"), "o").unwrap();
    let perms = Perms {
        read: true,
        write: false,
        share: true,
        admin: false,
    };
    h.add_user_scoped("ana@example.com", PW, "/", perms);
    let c = h.login("ana@example.com", PW).await;
    let res = Req::post("/api/shares")
        .cookie(&c)
        .json(json!({ "path": "other.txt", "expiresDays": 7, "password": "s3cret-pass", "download": true }))
        .send(&h.app)
        .await;
    let id = res.json["share"]["id"].as_str().unwrap().to_string();
    Req::patch(&format!("/api/shares/{id}"))
        .cookie(&c)
        .json(json!({ "download": false }))
        .send(&h.app)
        .await;
    Req::delete(&format!("/api/shares/{id}"))
        .cookie(&c)
        .send(&h.app)
        .await;
    let log = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    let line = |event: &str| {
        log.lines()
            .find(|l| l.contains(&format!(r#""event":"{event}""#)))
            .unwrap_or_else(|| panic!("{event} logged in {log}"))
            .to_string()
    };
    let created = line("share.create");
    assert!(created.contains(r#""password":true"#), "{created}");
    assert!(created.contains(r#""path":"/other.txt""#), "{created}");
    line("share.update");
    line("share.delete");
    assert!(!log.contains(&id), "the id is never logged");
    assert!(!log.contains("s3cret-pass"));
}

#[tokio::test]
async fn password_log() {
    let buf = LogBuf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let (h, dir) = harness_with_root(&[]);
    std::fs::write(dir.path().join("srv/other.txt"), "o").unwrap();
    h.add_user("admin@example.com", Some(PW), true);
    let c = h.login("admin@example.com", PW).await;
    let res = Req::post("/api/shares")
        .cookie(&c)
        .json(json!({ "path": "other.txt", "expiresDays": 7, "password": "s3cret-pass", "download": true }))
        .send(&h.app)
        .await;
    let id = res.json["share"]["id"].as_str().unwrap().to_string();
    for _ in 0..11 {
        Req::post(&format!("/s/{id}/unlock"))
            .json(json!({ "password": "wrong-one" }))
            .peer("203.0.113.5:4000")
            .send(&h.app)
            .await;
    }
    let log = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    let failed = log
        .lines()
        .find(|l| l.contains(r#""event":"share.password_failed""#))
        .expect("share.password_failed logged");
    assert!(failed.contains(r#""ip":"203.0.113.5""#), "{failed}");
    assert!(log.contains(r#""route":"share_password""#), "{log}");
    assert!(!log.contains("wrong-one") && !log.contains("s3cret-pass"));
    assert!(!log.contains(&id));
}
