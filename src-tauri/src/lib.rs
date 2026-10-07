#![recursion_limit = "512"]
// Byte-index string slicing panics inside a multi-byte character; cut with
// `rekindle_utils::text::{prefix, abbreviate}` instead (plan C2).
#![deny(clippy::string_slice)]

pub mod audit_repo;
pub mod audit_view;
pub mod channel_materialize;
pub mod channel_repo;
mod channels;
pub mod commands;
pub mod community_loader;
pub mod db;
pub mod db_helpers;
mod deep_links;
pub mod envelope_store_sqlite;
pub mod event_dispatch;
pub mod event_router;
pub mod friend_repo;
pub mod invite_helpers;
mod invoke;
pub mod keystore;
pub mod message_repo;
pub mod message_view;
mod os_notify;
mod platform;
pub mod serde_helpers;
mod services;
mod setup;
mod shortcuts;
mod shutdown;
pub mod signal_stores;
pub mod state;
pub mod state_helpers;
#[cfg(test)]
pub mod test_support;
mod tray;
pub mod video_channels;
pub mod window_labels;
mod windows;

use std::sync::Arc;

use tauri::{Manager, WindowEvent};

use state::{AppState, SharedState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    platform::linux_display_setup();

    // Suppress Veilid's noisy internal ERROR logs (e.g. "no compatible crypto
    // kinds in route" from stale route imports — our code handles these gracefully).
    // The veilid_api target only emits import/routing errors we already catch.
    // ANSI color only on a real terminal — fmt() defaults to ANSI even when
    // piped, which fills the dev-wrapper log files (scripts/linux-dev.sh,
    // scripts/mac-dev-app.sh) with escape codes.
    // Every line — Rekindle's and veilid-core's — passes through the
    // identifier scrubber before it is written (`rekindle_utils::log_scrub`).
    rekindle_utils::log_scrub::install_panic_hook();
    tracing_subscriber::fmt()
        .with_writer(rekindle_utils::log_scrub::ScrubbingMakeWriter::new(
            std::io::stdout,
        ))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,veilid_api=warn,veilid_core=warn")
            }),
        )
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .init();

    let shared_state: SharedState = Arc::new(AppState::default());
    let state_for_setup = Arc::clone(&shared_state);

    tauri::Builder::default()
        // MUST be first — prevents multiple instances
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Deep-link URLs in argv are handled by the single-instance
            // `deep-link` feature, which forwards them into the deep-link
            // plugin (firing `on_open_url`, see setup.rs) *before* this
            // callback runs. We only raise the existing window here — parsing
            // argv again would double-fire the deep link.
            if let Some(w) = app.get_webview_window(window_labels::BUDDY_LIST) {
                let _ = w.show();
                let _ = w.set_focus();
            } else if let Some(w) = app.get_webview_window(window_labels::LOGIN) {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        // NOTE: tauri_plugin_window_state removed — causes infinite windowDidMove
        // event loop on macOS when combined with prevent_exit(). See tauri#11489.
        .plugin(tauri_plugin_store::Builder::new().build())
        // NOTE: tauri-plugin-stronghold removed — the plugin was registered but
        // never invoked by the frontend, and its hardcoded production Argon2
        // params conflicted with our debug-mode params. `keystore.rs` is now
        // backed by `rekindle-vault` (SQLCipher, double-encrypted; ADR 0006),
        // not `iota_stronghold` at all — that dependency has been removed
        // from the workspace entirely.
        // The webview never opens links itself: `open_external_url`
        // confirms and opens https links from Rust, so the plugin's
        // injected `target=_blank` click handler stays off.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(move |app| setup::run(app, &state_for_setup))
        .manage(shared_state)
        .on_window_event(|window, event| {
            if let WindowEvent::Destroyed = event {
                if let Some(state) = window.app_handle().try_state::<SharedState>() {
                    state.event_router.remove(window.label());
                }
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let label = window.label();
                if label == window_labels::BUDDY_LIST {
                    // Hide buddy list to tray instead of closing
                    let _ = window.hide();
                    api.prevent_close();
                } else if label == window_labels::LOGIN {
                    // If login is closed and no buddy-list visible, exit the app
                    let has_buddy = window
                        .app_handle()
                        .get_webview_window(window_labels::BUDDY_LIST)
                        .and_then(|w| w.is_visible().ok())
                        .unwrap_or(false);
                    if !has_buddy {
                        window.app_handle().exit(0);
                    }
                }
                // Other windows (chat, settings, etc.) close normally
            }
        })
        .invoke_handler(invoke::handler())
        .build(tauri::generate_context!())
        .expect("error while building Rekindle")
        .run(|app_handle, event| match &event {
            tauri::RunEvent::ExitRequested { code, api, .. } if code.is_none() => {
                // code: None  = all windows closed (keep alive for tray icon)
                // code: Some  = programmatic exit via app.exit() (let it proceed)
                api.prevent_exit();
            }
            tauri::RunEvent::Exit => {
                shutdown::handle_exit(app_handle);
            }
            _ => {}
        });
}
