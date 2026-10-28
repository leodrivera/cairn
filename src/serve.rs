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
    runtime.block_on(serve(config))
}

async fn serve(config: Config) -> ExitCode {
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

    let app = router(AppState {
        config: Arc::new(config),
    });
    serve_on(listener, app, Limits::DEFAULT, stop_signal()).await;
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
}

impl Limits {
    /// `docker stop` waits 10 s before SIGKILL; 3 s of grace leaves room.
    pub const DEFAULT: Limits = Limits {
        header_read: Duration::from_secs(10),
        grace: Duration::from_secs(3),
    };
}

/// Serves HTTP/1.1 on `listener` until `stop` completes, then waits at most `limits.grace` for open connections.
pub async fn serve_on(
    listener: TcpListener,
    app: Router,
    limits: Limits,
    stop: impl Future<Output = ()>,
) {
    let graceful = GracefulShutdown::new();
    let service = TowerToHyperService::new(app);
    let mut stop = std::pin::pin!(stop);
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let stream = match accepted {
                    Ok((stream, _)) => stream,
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
                    .serve_connection(TokioIo::new(stream), service.clone());
                let conn = graceful.watch(conn);
                tokio::spawn(async move {
                    let _ = conn.await;
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
            grace: Duration::from_secs(3),
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

    #[tokio::test]
    async fn shutdown_has_a_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let limits = Limits {
            header_read: Duration::from_secs(30),
            grace: Duration::from_millis(200),
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
}
