//! Trash log lines. Alone in its own test binary: a scoped subscriber races with other tests on the same callsites.

mod common;

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

fn lines(buf: &LogBuf) -> Vec<serde_json::Value> {
    String::from_utf8(buf.0.lock().unwrap().clone())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[tokio::test]
async fn trash_log_events() {
    let buf = LogBuf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    // purge_logs_count
    let (h, dir) = harness_with_root(&[]);
    let root = dir.path().join("srv");
    h.add_user("admin@example.com", Some(PW), true);
    let c = h.login("admin@example.com", PW).await;
    for name in ["a.txt", "b.txt"] {
        std::fs::write(root.join(name), "x").unwrap();
        Req::delete("/api/files")
            .cookie(&c)
            .json(json!({ "paths": [name] }))
            .send(&h.app)
            .await;
    }
    cairn::trash::ops::purge(&h.state.trash, &h.state.db, cairn::now() + 31 * 86_400);
    let purged: Vec<_> = lines(&buf)
        .into_iter()
        .filter(|l| l["event"] == "trash.purged")
        .collect();
    assert_eq!(purged.len(), 1, "{purged:?}");
    assert_eq!(purged[0]["count"], 2);

    // taken_folder_logged
    let dir0 = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir0.path().join("srv/.cairn-trash")).unwrap();
    let _h = common::harness_at(dir0, &[]);
    let taken: Vec<_> = lines(&buf)
        .into_iter()
        .filter(|l| l["event"] == "startup.trash_folder_taken")
        .collect();
    assert_eq!(taken.len(), 1, "{taken:?}");
    assert_eq!(taken[0]["folder"], "/.cairn-trash");
}
