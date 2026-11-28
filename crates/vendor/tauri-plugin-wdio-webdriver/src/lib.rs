// Rekindle vendors only the macOS and Linux back ends (see README.md, change
// 3); Windows is driven by tauri-driver, which needs no in-app plugin. On
// every other target this crate is empty, so `cargo check --workspace` passes
// there; `rekindle-desktop` refuses its `webdriver-test` feature on those
// targets (`src-tauri/src/lib.rs`).
#![cfg(any(target_os = "macos", target_os = "linux"))]

use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

// A loopback remote control of every window: debug builds only (Rekindle
// patch; `cargo xtask check-webdriver-release`).
#[cfg(not(debug_assertions))]
compile_error!("tauri-plugin-wdio-webdriver is a test driver and never builds in release");

mod error;
#[cfg(target_os = "macos")]
mod eval_channel;
#[cfg(desktop)]
mod platform;
#[cfg(desktop)]
mod server;
#[cfg(desktop)]
mod webdriver;

pub use error::{Error, Result};

/// Default port for the `WebDriver` HTTP server
pub const DEFAULT_PORT: u16 = 4445;

/// Environment variable name for configuring the port
pub const PORT_ENV_VAR: &str = "TAURI_WEBDRIVER_PORT";

/// Initializes the plugin with default settings.
///
/// The port is determined in the following order:
/// 1. `TAURI_WEBDRIVER_PORT` environment variable (if set and valid)
/// 2. Default port (4445)
#[must_use]
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    let port = std::env::var(PORT_ENV_VAR)
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);

    init_with_port(port)
}

/// Initializes the plugin with a custom port.
///
/// This ignores the `TAURI_WEBDRIVER_PORT` environment variable.
#[cfg(desktop)]
#[must_use]
pub fn init_with_port<R: Runtime>(port: u16) -> TauriPlugin<R> {
    init_with_routes(port, |_| axum::Router::new())
}

/// Mount app-specific test routes behind the same loopback request guard.
#[cfg(desktop)]
pub fn init_with_routes<R: Runtime>(
    port: u16,
    routes: impl FnOnce(tauri::AppHandle<R>) -> axum::Router + Send + 'static,
) -> TauriPlugin<R> {
    use tauri::Manager;

    Builder::new("wdio-webdriver")
        .setup(move |app, _api| {
            // Manage per-window alert state
            app.manage(platform::AlertStateManager::default());

            // Arc so the (non-generic) objc2 message handler can hold its own clone; see eval_channel.
            #[cfg(target_os = "macos")]
            app.manage(std::sync::Arc::new(
                eval_channel::EvalResultRegistry::default(),
            ));

            // Start the macOS headless run-loop pump early (before any webview loads); Once-guarded,
            // so the on_webview_ready registration remains a fallback. See #540.
            #[cfg(target_os = "macos")]
            platform::start_runloop_pump_early(app.app_handle());

            // Start the WebDriver HTTP server
            let app_handle = app.app_handle().clone();
            server::start(app_handle.clone(), port, routes(app_handle));
            tracing::info!("WDIO WebDriver plugin initialized on port {port}");

            Ok(())
        })
        .on_webview_ready(|webview| {
            platform::register_webview_handlers(&webview);
        })
        .build()
}

/// Initializes the plugin with a custom port.
///
/// On Android and iOS the plugin registers but does nothing: the embedded driver is desktop-only.
#[cfg(mobile)]
#[must_use]
pub fn init_with_port<R: Runtime>(_port: u16) -> TauriPlugin<R> {
    Builder::new("wdio-webdriver")
        .setup(|_app, _api| {
            tracing::warn!("tauri-plugin-wdio-webdriver is desktop-only; not started on mobile");
            Ok(())
        })
        .build()
}
