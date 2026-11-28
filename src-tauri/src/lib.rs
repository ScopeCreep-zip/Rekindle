#![recursion_limit = "512"]
// Byte-index string slicing panics inside a multi-byte character; cut with
// `rekindle_utils::text::{prefix, abbreviate}` instead (plan C2).
#![deny(clippy::string_slice)]

mod channels;
pub mod commands;
pub mod db;
pub mod db_helpers;
mod deep_links;
pub mod event_dispatch;
pub mod event_router;
pub mod event_snapshot;
pub mod host_call;
mod invoke;
pub mod keystore;
mod os_notify;
mod platform;
mod portal_shortcuts;
mod services;
mod setup;
/// The port ledger: which Tauri commands are Host calls, the shell's own,
/// or waiting for their port (plan E1, recipe step 6). Its test is the
/// check, so it builds with the tests.
#[cfg(test)]
mod shell_commands;
mod shortcuts;
mod shutdown;
mod skin_scheme;
mod skin_windows;
pub mod state;
pub mod state_helpers;
mod stop_signals;
#[cfg(test)]
pub mod test_support;
mod tray;
pub mod video_channels;
#[cfg(all(feature = "webdriver-test", debug_assertions))]
mod webdriver;
pub mod window_labels;
mod windows;

// The WebDriver walk's driver serves a local remote control of every window
// (plan E7.13): a release build never carries it
// (`cargo xtask check-webdriver-release`).
#[cfg(all(feature = "webdriver-test", not(debug_assertions)))]
compile_error!("the webdriver-test feature is for debug builds only");

// The vendored driver carries only the macOS and Linux back ends and is an
// empty crate elsewhere (its README, change 3); Windows is driven by
// tauri-driver, which needs no in-app plugin.
#[cfg(all(
    feature = "webdriver-test",
    not(any(target_os = "macos", target_os = "linux"))
))]
compile_error!("the vendored tauri-plugin-wdio-webdriver supports macOS and Linux only");

pub use services::shell_session::ShellSession;

use std::sync::Arc;

use tauri::{Manager, WindowEvent};

use state::{AppState, SharedState};

/// The single-instance plugin. It MUST be registered first: a second
/// launch raises this app's window and exits.
#[cfg(not(all(feature = "webdriver-test", debug_assertions)))]
fn single_instance() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_single_instance::init(|app, _args, _cwd| {
        // Deep-link URLs in argv are handled by the single-instance
        // `deep-link` feature, which forwards them into the deep-link
        // plugin (firing `on_open_url`, see setup.rs) *before* this
        // callback runs. We only raise the existing window here — parsing
        // argv again would double-fire the deep link.
        if let Some(w) = app
            .get_webview_window(window_labels::BUDDY_LIST)
            .or_else(|| app.get_webview_window(window_labels::LOGIN))
        {
            crate::windows::reveal_window(&w);
        }
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Before anything holds a secret: the in-process Host holds the same
    // keys as `rekindled` until F1 (plan E7.1, roadmap 5.1). A process that
    // cannot be hardened does not run.
    if let Err(e) = rekindle_node::host::hardening::forbid_core_dumps() {
        // Logging is not set up yet; stderr is the one channel. A closed
        // stderr leaves nothing else to report to.
        let _ = std::io::Write::write_fmt(
            &mut std::io::stderr(),
            format_args!("rekindle: cannot forbid core dumps: {e}\n"),
        );
        std::process::exit(1);
    }
    #[cfg(target_os = "linux")]
    platform::linux_display_setup();

    // Rekindle's own targets at info, everything else (Veilid included) at
    // warn: the daemon's filter (`rekindle-node/src/host/mod.rs`). Veilid's
    // log targets are facilities, not its crate path (`veilid_log!` uses the
    // module's `impl_veilid_log_facility!`, 0.5.8 `veilid-core/src/logging/
    // macros.rs:6-11`: `rtab`, `stor`, `net`, `rpc`, `tstore`, `fanout`,
    // `attach`, `crypto`, …), so a `veilid_core=` directive matches nothing.
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
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("rekindle=info,warn")),
        )
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .init();

    let shared_state: SharedState = Arc::new(AppState::default());
    let state_for_setup = Arc::clone(&shared_state);

    let builder = tauri::Builder::default();
    // The walk's build skips the single-instance plugin: it runs beside the
    // user's own Rekindle on a separate data root, and must neither hand its
    // launch to that app nor take the user's launches (`webdriver.rs`).
    #[cfg(all(feature = "webdriver-test", debug_assertions))]
    let builder = builder.plugin(webdriver::plugin());
    #[cfg(not(all(feature = "webdriver-test", debug_assertions)))]
    let builder = builder.plugin(single_instance());
    builder
        // NOTE: tauri_plugin_window_state removed — causes infinite windowDidMove
        // event loop on macOS when combined with prevent_exit(). See tauri#11489.
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
        // The skin package files, from the Host (plan E7.7.2).
        .register_asynchronous_uri_scheme_protocol(skin_scheme::SCHEME, |ctx, request, responder| {
            skin_scheme::handle(&ctx, &request, responder);
        })
        .setup(move |app| setup::run(app, &state_for_setup))
        .manage(shared_state)
        .on_window_event(|window, event| {
            skin_windows::on_window_event(window, event);
            windows::popup::on_window_event(window, event);
            if let WindowEvent::Destroyed = event {
                if let Some(state) = window.app_handle().try_state::<SharedState>() {
                    state.event_router.remove(window.label());
                    // Plan CL2: its video frame channels go with it.
                    state.video_channels.release_window(window.label());
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
