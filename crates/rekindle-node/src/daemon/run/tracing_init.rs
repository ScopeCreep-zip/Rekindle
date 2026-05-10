//! Daemon tracing initialization with runtime-adjustable log level.
//!
//! Constructs a `tracing_subscriber` stack with a `reload::Layer` that
//! allows the active `EnvFilter` to be swapped at runtime without restart.
//! The reload handle is stored in `signals::register_reload_handle()` for
//! access from the SIGUSR2 signal handler.
//!
//! The CLI must NOT initialize its own tracing subscriber before calling
//! `run_daemon()` — only one global default subscriber is allowed per process.
//! The CLI's `node start` subcommand skips its own tracing init and defers
//! to this module.

use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, reload, EnvFilter};

/// Initialize the daemon's tracing subscriber with runtime-adjustable level.
///
/// Reads the initial level from `RUST_LOG` env var if set, otherwise defaults
/// to `info`. The reload handle is registered globally for SIGUSR2 rotation.
///
/// # Panics
///
/// Panics if a global subscriber is already set (another tracing init ran first).
/// The CLI must NOT init tracing before calling `run_daemon()`.
pub fn init_tracing() {
    let default_filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());

    let filter_invalid = EnvFilter::try_new(&default_filter).is_err();
    let env_filter = EnvFilter::try_new(&default_filter)
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let (filter_layer, reload_handle) = reload::Layer::new(env_filter);

    let fmt_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(false)
        .with_line_number(false)
        .with_ansi(atty::is(atty::Stream::Stderr));

    let subscriber = tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer);

    tracing::subscriber::set_global_default(subscriber)
        .expect("tracing subscriber already set — CLI must not init tracing for 'node start'");

    // Store the reload handle for SIGUSR2 log level rotation.
    super::signals::register_reload_handle(reload_handle);

    if filter_invalid {
        tracing::warn!(
            rust_log = %default_filter,
            "invalid RUST_LOG value — falling back to 'info'"
        );
    }

    tracing::info!(filter = %default_filter, "tracing initialized with reload capability");
}
