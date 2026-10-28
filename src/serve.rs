//! `cairn serve`: check the configuration and the folders, then serve until SIGTERM or Ctrl-C.

use std::process::ExitCode;
use std::sync::Arc;

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
    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(stop_signal())
        .await
    {
        tracing::error!(event = "server.error", error = %e);
        return ExitCode::FAILURE;
    }
    tracing::info!(event = "shutdown");
    ExitCode::SUCCESS
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
