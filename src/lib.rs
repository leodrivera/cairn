//! Cairn: a self-hosted web file browser. The binary in `main.rs` is a thin shell over this library.

pub mod auth;
pub mod cli;
pub mod config;
pub mod db;
pub mod health;
pub mod http;
pub mod log;
pub mod serve;
pub mod startup;
pub mod user_cli;
pub mod users;

/// Unix seconds. Store functions take the time as an argument, so tests can move the clock.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
