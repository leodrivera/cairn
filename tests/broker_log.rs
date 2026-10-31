//! The broker's unreachable log line. Alone in its own test binary: other tests hitting the same log callsite on
//! other threads race with the scoped subscriber this test installs, and the line is then lost.

mod common;

use cairn::auth::broker::Broker;
use cairn::config::{Config, Vars};
use common::broker::FakeBroker;
use serde_json::json;

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
async fn unreachable_log_names_the_api() {
    let buf = LogBuf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let fake = FakeBroker::start(json!([])).await;
    fake.set_list_status(404);
    let vars: Vars = [
        ("CAIRN_AUTH_METHODS", "oauth"),
        ("CAIRN_PUBLIC_URL", "https://files.example.com"),
        ("CAIRN_OAUTH_BROKER_URL", fake.url.as_str()),
        ("CAIRN_OAUTH_SECRET", "0123456789abcdef0123456789abcdef"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let config = Config::from_vars(&vars).unwrap();
    Broker::new(config.auth.oauth.as_ref().unwrap())
        .refresh()
        .await;
    let log = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    let line = log
        .lines()
        .find(|l| l.contains("oauth.broker_unreachable"))
        .expect("logged");
    assert!(line.contains("Tinyauth v5"), "{line}");
}
