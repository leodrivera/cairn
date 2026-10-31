//! Grant log lines. Alone in its own test binary: a scoped subscriber races with other tests on the same callsites.

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
async fn grant_log_events() {
    let buf = LogBuf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let (h, dir) = harness_with_root(&[]);
    std::fs::create_dir_all(dir.path().join("srv/photos")).unwrap();
    let perms = Perms {
        read: true,
        write: true,
        share: true,
        admin: false,
    };
    h.add_user_scoped("ana@example.com", PW, "/", perms);
    h.add_user_scoped("bob@example.com", PW, "/", perms);
    let c = h.login("ana@example.com", PW).await;
    // unknown_email_logged_as_count
    let res = Req::post("/api/grants")
        .cookie(&c)
        .json(json!({ "path": "photos", "emails": ["bob@example.com", "typed-one@example.com", "typed-two@example.com"], "permission": "read" }))
        .send(&h.app)
        .await;
    let id = res.json["results"][0]["grant"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    // rate_limited_logged
    let many: Vec<String> = (0..20).map(|i| format!("n{i}@example.com")).collect();
    Req::post("/api/grants")
        .cookie(&c)
        .json(json!({ "path": "photos", "emails": many, "permission": "read" }))
        .send(&h.app)
        .await;
    let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let unknown: Vec<_> = lines
        .iter()
        .filter(|l| l["event"] == "grant.unknown_email")
        .collect();
    assert_eq!(unknown[0]["count"], 2, "{unknown:?}");
    assert!(
        !text.contains("typed-one") && !text.contains("typed-two"),
        "typed e-mails were logged"
    );
    assert!(!text.contains(&id), "a grant id was logged");
    assert!(lines.iter().any(|l| l["event"] == "grant.create"));
    let limited: Vec<_> = lines
        .iter()
        .filter(|l| l["event"] == "http.rate_limited")
        .collect();
    assert_eq!(limited.len(), 1, "{limited:?}");
    assert_eq!(limited[0]["route"], "grant_lookup");
}
