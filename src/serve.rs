//! `cairn serve`: check the configuration and the folders, then serve until SIGTERM or Ctrl-C.

use std::future::Future;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use crate::config::{CaptchaConfig, CaptchaVendor, Config};
use crate::http::{AppState, router};

/// Prints every configuration problem, one per line, and returns exit code 2.
pub fn report_invalid(errors: &[String]) -> ExitCode {
    for e in errors {
        eprintln!("{e}");
    }
    crate::log::init(crate::config::LogLevel::Info);
    tracing::error!(event = "startup.config_invalid", count = errors.len());
    ExitCode::from(2)
}

pub fn run(config: Config) -> ExitCode {
    crate::log::init(config.log);
    let problems = crate::startup::check_paths(&config);
    if !problems.is_empty() {
        return report_invalid(&problems);
    }
    let db = match crate::db::Db::open(&crate::user_cli::db_path(&config)) {
        Ok(db) => db,
        Err(message) => return report_invalid(&[message]),
    };
    crate::fs::upload::cleanup_leftovers(&db);
    if crate::users::count(&db).unwrap_or(0) == 0 {
        tracing::warn!(
            event = "startup.no_users",
            hint = "docker exec -it cairn /cairn user add \"Admin\" admin@example.com --admin --password",
        );
    }

    let runtime = if config.threads == 1 {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
    } else {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(config.threads)
            .enable_all()
            .build()
    };
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("Cannot start the runtime ({e}).");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(serve(config, db))
}

async fn serve(config: Config, db: crate::db::Db) -> ExitCode {
    let addr = config.addr;
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("CAIRN_ADDR: cannot listen on {addr} ({e}). Pick a free address and port.");
            return ExitCode::FAILURE;
        }
    };

    let mut methods = Vec::new();
    if config.auth.password {
        methods.push("password");
    }
    if config.auth.oauth.is_some() {
        methods.push("oauth");
    }
    if !config.auth.oidc.is_empty() {
        methods.push("oidc");
    }
    let captcha = match &config.captcha {
        CaptchaConfig::None => "none",
        CaptchaConfig::On {
            vendor: CaptchaVendor::Recaptcha,
            ..
        } => "recaptcha",
        CaptchaConfig::On {
            vendor: CaptchaVendor::Turnstile,
            ..
        } => "turnstile",
        CaptchaConfig::On {
            vendor: CaptchaVendor::Hcaptcha,
            ..
        } => "hcaptcha",
    };
    tracing::info!(
        event = "startup.listening",
        addr = %addr,
        auth_methods = methods.join(","),
        captcha,
        threads = config.threads,
        version = env!("CARGO_PKG_VERSION"),
    );
    if config.auth.password && matches!(config.captcha, CaptchaConfig::None) {
        tracing::warn!(
            event = "startup.captcha_disabled",
            reason = "password sign-in without CAPTCHA",
            hint = "set CAIRN_CAPTCHA before you open Cairn to the internet",
        );
    }

    let state = AppState::new(config, db);
    spawn_trash_upkeep(&state);
    // Never awaited: the socket opens before any provider or broker answers.
    if let Some(broker) = state.broker.clone() {
        tokio::spawn(async move { broker.refresh().await });
    }
    state.oidc.spawn_retry();
    let app = router(state);
    let open_files = rustix::process::getrlimit(rustix::process::Resource::Nofile).current;
    let limits = Limits {
        max_connections: connection_cap(open_files.unwrap_or(u64::MAX)),
        ..Limits::DEFAULT
    };
    serve_on(listener, app, limits, stop_signal()).await;
    tracing::info!(event = "shutdown");
    ExitCode::SUCCESS
}

/// Time limits that keep a slow or stuck client from holding Cairn.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// A request head must arrive in full within this time, or the connection closes.
    pub header_read: Duration,
    /// After the stop signal, open requests get this long to finish; then Cairn exits anyway.
    pub grace: Duration,
    /// Connections served at once. Past this they wait in the listen queue until one closes.
    pub max_connections: usize,
}

impl Limits {
    /// `docker stop` waits 10 s before SIGKILL; 3 s of grace leaves room.
    pub const DEFAULT: Limits = Limits {
        header_read: Duration::from_secs(10),
        grace: Duration::from_secs(3),
        max_connections: 512,
    };
}

/// `Limits::DEFAULT`'s cap, or a third of what the process may have open: a download holds its socket and its file,
/// and the database and logs need some too.
pub fn connection_cap(open_files: u64) -> usize {
    let third = usize::try_from(open_files / 3).unwrap_or(usize::MAX);
    third.clamp(1, Limits::DEFAULT.max_connections)
}

/// The router, with the connection's peer address on every request.
fn with_peer(
    app: Router,
    peer: std::net::SocketAddr,
) -> TowerToHyperService<
    tower::util::MapRequest<
        Router,
        impl FnMut(hyper::Request<hyper::body::Incoming>) -> hyper::Request<hyper::body::Incoming>
        + Clone,
    >,
> {
    TowerToHyperService::new(tower::ServiceExt::map_request(
        app,
        move |mut req: hyper::Request<hyper::body::Incoming>| {
            req.extensions_mut().insert(crate::http::client::Peer(peer));
            req
        },
    ))
}

/// Serves HTTP/1.1 on `listener` until `stop` completes, then waits at most `limits.grace` for open connections.
pub async fn serve_on(
    listener: TcpListener,
    app: Router,
    limits: Limits,
    stop: impl Future<Output = ()>,
) {
    let graceful = GracefulShutdown::new();
    let places = Arc::new(Semaphore::new(limits.max_connections));
    let mut stop = std::pin::pin!(stop);
    loop {
        let place = tokio::select! {
            place = places.clone().acquire_owned() => place.expect("the semaphore is never closed"),
            () = &mut stop => break,
        };
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = match accepted {
                    Ok(pair) => pair,
                    Err(e) => {
                        // Out of file descriptors and similar: back off instead of spinning.
                        tracing::warn!(event = "http.accept_failed", error = %e);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let conn = http1::Builder::new()
                    .timer(TokioTimer::new())
                    .header_read_timeout(limits.header_read)
                    .serve_connection(TokioIo::new(stream), with_peer(app.clone(), peer));
                let conn = graceful.watch(conn);
                tokio::spawn(async move {
                    let _ = conn.await;
                    drop(place);
                });
            }
            () = &mut stop => break,
        }
    }
    drop(listener);
    tokio::select! {
        () = graceful.shutdown() => {}
        () = tokio::time::sleep(limits.grace) => {
            tracing::warn!(event = "shutdown.forced", grace_ms = limits.grace.as_millis() as u64);
        }
    }
}

/// SIGTERM (`docker stop`) or Ctrl-C.
async fn stop_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("SIGTERM handler");
    tokio::select! {
        _ = ctrl_c => {}
        _ = term.recv() => {}
    }
}

/// Reconciles and purges the trash once now, then every hour, off the async threads.
fn spawn_trash_upkeep(state: &AppState) {
    let (trash, db) = (state.trash.clone(), state.db.clone());
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            // The first tick is immediate, so this also runs at start.
            tick.tick().await;
            let (trash, db) = (trash.clone(), db.clone());
            let _ = tokio::task::spawn_blocking(move || {
                crate::trash::ops::reconcile(&trash, &db);
                crate::trash::ops::purge(&trash, &db, crate::now());
            })
            .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    async fn half_sent_request(addr: std::net::SocketAddr) -> TcpStream {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: x\r\n")
            .await
            .unwrap();
        stream
    }

    #[tokio::test]
    async fn slow_header_is_dropped() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let limits = Limits {
            header_read: Duration::from_millis(200),
            ..Limits::DEFAULT
        };
        tokio::spawn(serve_on(
            listener,
            axum::Router::new(),
            limits,
            std::future::pending(),
        ));

        let mut stream = half_sent_request(addr).await;
        let mut buf = Vec::new();
        let closed =
            tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut buf)).await;
        assert!(
            closed.is_ok(),
            "connection still open 2 s after a 200 ms header timeout"
        );
    }

    #[test]
    fn the_cap_follows_the_open_file_limit() {
        assert_eq!(connection_cap(1_048_576), 512);
        assert_eq!(connection_cap(1024), 341);
        assert_eq!(connection_cap(64), 21);
        assert_eq!(connection_cap(1), 1);
    }

    #[tokio::test]
    async fn past_the_cap_a_connection_waits_for_a_free_place() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let limits = Limits {
            header_read: Duration::from_secs(30),
            max_connections: 2,
            ..Limits::DEFAULT
        };
        tokio::spawn(serve_on(
            listener,
            axum::Router::new(),
            limits,
            std::future::pending(),
        ));

        let first = half_sent_request(addr).await;
        let _second = half_sent_request(addr).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut third = TcpStream::connect(addr).await.unwrap();
        third
            .write_all(b"GET / HTTP/1.0\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut reply = [0u8; 16];
        let wait = Duration::from_millis(300);
        let early = tokio::time::timeout(wait, third.read(&mut reply)).await;
        assert!(early.is_err(), "answered past the cap");

        drop(first);
        let late = tokio::time::timeout(Duration::from_secs(2), third.read(&mut reply)).await;
        assert!(late.is_ok(), "no answer after a place came free");
    }

    #[tokio::test]
    async fn an_idle_connection_gives_its_place_back() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let limits = Limits {
            header_read: Duration::from_millis(200),
            ..Limits::DEFAULT
        };
        tokio::spawn(serve_on(
            listener,
            axum::Router::new(),
            limits,
            std::future::pending(),
        ));

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        let closed =
            tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut buf)).await;
        assert!(closed.is_ok(), "kept open 2 s after a 200 ms limit");
    }

    #[tokio::test]
    async fn shutdown_has_a_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let limits = Limits {
            header_read: Duration::from_secs(30),
            grace: Duration::from_millis(200),
            ..Limits::DEFAULT
        };
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve_on(listener, axum::Router::new(), limits, async {
            let _ = stopped.await;
        }));

        let _open = half_sent_request(addr).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        stop.send(()).unwrap();
        let done = tokio::time::timeout(Duration::from_secs(1), server).await;
        assert!(
            done.is_ok(),
            "server still running 1 s after stop with a 200 ms grace period"
        );
    }

    #[tokio::test]
    async fn peer_is_inserted() {
        use crate::http::client::Peer;
        use axum::Extension;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/",
            axum::routing::get(|Extension(Peer(peer)): Extension<Peer>| async move {
                peer.ip().to_string()
            }),
        );
        tokio::spawn(serve_on(
            listener,
            app,
            Limits::DEFAULT,
            std::future::pending(),
        ));

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"GET / HTTP/1.0\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.unwrap();
        assert!(
            reply.starts_with("HTTP/1.0 200") || reply.starts_with("HTTP/1.1 200"),
            "{reply}"
        );
        assert!(reply.ends_with("127.0.0.1"), "{reply}");
    }
}
