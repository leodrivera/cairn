//! Logs: one JSON object per line on stdout, so `docker logs` is the log store.

use tracing::Level;

use crate::config::LogLevel;

pub fn init(level: LogLevel) {
    let level = match level {
        LogLevel::Debug => Level::DEBUG,
        LogLevel::Info => Level::INFO,
        LogLevel::Warn => Level::WARN,
        LogLevel::Error => Level::ERROR,
    };
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_target(false)
        .with_current_span(false)
        .with_span_list(false)
        .with_max_level(level)
        .with_writer(std::io::stdout)
        .init();
}
