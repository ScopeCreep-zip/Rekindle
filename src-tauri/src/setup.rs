//! App setup: runs once during `tauri::Builder::setup`, wiring state, storage,
//! background services, and the Veilid node.

use std::sync::Arc;

use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;

use crate::state::SharedState;
use crate::{event_dispatch, services, shortcuts, tray};

/// Grant the debug-only commands (compiled out of release) to their
/// windows. The capability files live in `capabilities-dev/`, outside the
/// `capabilities/` directory `tauri-build` validates, so a release build
/// neither embeds nor can resolve them.
#[cfg(debug_assertions)]
fn add_dev_capabilities(app: &tauri::App) -> tauri::Result<()> {
    app.add_capability(include_str!("../capabilities-dev/dev-settings.json"))?;
    app.add_capability(include_str!("../capabilities-dev/dev-community.json"))
}

/// Run all one-time app setup. Invoked from the `tauri::Builder::setup` closure.
pub fn run(app: &tauri::App, state: &SharedState) -> Result<(), Box<dyn std::error::Error>> {
    // Store app handle in AppState so background services can emit events
    *state.app_handle.write() = Some(app.handle().clone());

    // The data root every host shares (plan C5): the same folders Tauri's
    // `app_{data,config}_dir` name, created owner-only. The host claims it
    // (plan D3.1): its node lock makes the desktop and `rekindled` refuse to
    // run one node twice on a root (F3), and the layout check runs under it
    // before anything opens a store (plan D1). The claim is held by the
    // host for the process's lifetime.
    let root = rekindle_db::paths::DataRoot::resolve()?;
    let claimed = match rekindle_node::host::claim(root.clone()) {
        Ok(claimed) => claimed,
        Err(e) => {
            tracing::error!(error = %e, "data root claim failed");
            show_startup_error(app, &format!("{e:#}"));
            return Ok(());
        }
    };

    #[cfg(debug_assertions)]
    add_dev_capabilities(app)?;
    // The first window. It is built here rather than declared in
    // `tauri.conf.json` so it passes through `windows::base_builder` and
    // its navigation guard like every other window.
    crate::windows::open_login(app.handle(), None)?;

    tray::setup_tray(app)?;

    // Deep links — one unified path for all desktop platforms.
    //
    // Delivery differs by OS but converges on `on_open_url`:
    //   - macOS: Apple Event while the app is running (warm start).
    //   - Windows/Linux: the OS re-launches the exe with the URL in argv;
    //     the single-instance `deep-link` feature forwards that argv into
    //     the deep-link plugin, which fires `on_open_url` here.
    //
    // Linux/Windows also need the `rekindle://` scheme registered with the
    // desktop at runtime (handled at install time by the bundler, but
    // `register_all` covers AppImage / portable / dev runs that skip that).
    #[cfg(any(windows, target_os = "linux"))]
    if let Err(e) = app.deep_link().register_all() {
        tracing::warn!(error = %e, "failed to register rekindle:// deep-link scheme");
    }

    let dl_handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            crate::deep_links::handle_deep_link_url(&dl_handle, url.as_str());
        }
    });

    // Cold start: the app was launched *by* a deep link. The plugin parsed
    // argv at init (before any listener existed), so `on_open_url` won't
    // fire for it — drain the captured URL(s) here instead.
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        let cold_handle = app.handle().clone();
        for url in urls {
            crate::deep_links::handle_deep_link_url(&cold_handle, url.as_str());
        }
    }

    // Register global keyboard shortcuts (plugin registered here for state access)
    shortcuts::register(app, state)?;

    // Resolve the Lost Cargo cache root (per-community sub-dirs are
    // created on first use). Stays None if the platform doesn't expose
    // an app data dir — file uploads will fail clearly in that case.
    let cache_root = root.file_cache();
    if let Err(e) = std::fs::create_dir_all(&cache_root) {
        tracing::warn!(error = %e, path = %cache_root.display(), "failed to create file_cache dir");
    } else {
        *state.file_cache_root.write() = Some(cache_root);
    }

    app.manage(root);

    // The identity's vault handle, filled at unlock and emptied at lock
    // (`host::identity`). AppState owns the same handle so services with
    // `&AppState` reach the vault without threading Tauri State through
    // every callsite.
    app.manage(state.keystore.clone());

    // Phase 5 — drive the lifecycle FSM through boot. Stopped → Starting
    // marks "Veilid attach beginning"; the dispatch loop transitions to
    // Locked once attach completes (see services/veilid/lifecycle/status.rs).
    let _ = state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Starting);

    // Phase 5 — forward lifecycle transitions to the frontend so the UI
    // can render state badges ("Reconnecting…", "Locked", etc.) live
    // instead of polling lifecycle_current. One spawned task per
    // process lifetime; ends when the broadcast sender drops.
    {
        let mut rx = state.lifecycle.subscribe();
        let app_for_lifecycle = app.handle().clone();
        // Tauri's `setup` closure runs on the main thread with no ambient
        // Tokio reactor — spawn on Tauri's managed runtime (same as the
        // Veilid task below), not bare `tokio::spawn`.
        tauri::async_runtime::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(next) => {
                        event_dispatch::emit(
                            &app_for_lifecycle,
                            event_dispatch::WebviewEvent::Lifecycle {
                                state: next,
                                at_ms: rekindle_utils::timestamp_ms_i64(),
                            },
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // Buffer overflowed (>64 queued transitions). Log
                        // and continue; the FSM state is still authoritative
                        // via lifecycle_current() and the next live
                        // transition will resync the frontend.
                        tracing::warn!(
                            missed = n,
                            "lifecycle-event forwarder lagged; missed {n} transitions",
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // Sender dropped → app shutting down. Exit cleanly.
                        tracing::debug!("lifecycle-event forwarder exiting (channel closed)");
                        break;
                    }
                }
            }
        });
    }

    // Note: game detection starts after login (in start_background_services)
    // to avoid burning CPU before user is authenticated.

    // Start the Veilid node at app startup — the node lives for the
    // entire app lifetime. User login/logout is independent of node lifecycle.
    let state_for_veilid = Arc::clone(state);
    let app_handle_clone = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        match services::veilid::initialize_node(&app_handle_clone, &state_for_veilid).await {
            Ok(update_rx) => {
                // The host runs in-process over the adopted node (plan
                // D3.1) and serves the bus the TUI and CLI attach to.
                if let Err(e) = start_host(&state_for_veilid, claimed).await {
                    tracing::error!(error = %e, "host did not start");
                    exit_with_error(&app_handle_clone, &e);
                    return;
                }
                // Create shutdown channel for the dispatch loop
                let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel(1);
                *state_for_veilid.shutdown_tx.write() = Some(shutdown_tx);

                // Start dispatch loop (runs until app exit)
                let dispatch_state = Arc::clone(&state_for_veilid);
                let dispatch_handle = tokio::spawn(services::veilid::start_dispatch_loop(
                    app_handle_clone,
                    dispatch_state,
                    update_rx,
                    shutdown_rx,
                ));
                *state_for_veilid.dispatch_loop_handle.write() = Some(dispatch_handle);
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to start Veilid node at app startup");
                exit_with_error(&app_handle_clone, &e);
            }
        }
    });

    // The single event-dispatch task: every webview event goes through
    // it to the windows in its audience (`event_router`).
    event_dispatch::spawn_dispatch_loop(state);

    tracing::info!("Rekindle started");
    Ok(())
}

/// Start the in-process host over the node `initialize_node` adopted.
async fn start_host(
    state: &SharedState,
    claimed: rekindle_node::host::ClaimedRoot,
) -> Result<(), String> {
    let node = state
        .transport
        .read()
        .clone()
        .ok_or("transport node not adopted")?;
    let host = rekindle_node::host::Host::start(rekindle_node::host::HostConfig {
        root: claimed,
        stores: state.host_stores(),
        transport: rekindle_node::host::TransportSource::Adopted(node),
        serve_bus: true,
        clock: Arc::clone(&state.clock),
        rng: Arc::clone(&state.rng),
    })
    .await
    .map_err(|e| format!("{e:#}"))?;
    if let Some(app) = state.app_handle.read().clone() {
        crate::event_dispatch::spawn_host_stream(app, state, Arc::clone(&host));
    }
    state.host.set(host);
    Ok(())
}

/// Show a failure after the first window opened, and exit when the dialog
/// closes: without its node and host the app can do nothing.
fn exit_with_error(app: &tauri::AppHandle, detail: &str) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
    let handle = app.clone();
    app.dialog()
        .message(format!(
            "Rekindle could not start and has to close.\n\n{detail}"
        ))
        .title("Rekindle could not start")
        .kind(MessageDialogKind::Error)
        .show(move |_| handle.exit(1));
}

/// Show a startup failure in a native dialog and exit when it closes. No
/// window is open yet, so the dialog has no parent.
fn show_startup_error(app: &tauri::App, detail: &str) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
    let handle = app.handle().clone();
    app.dialog()
        .message(format!(
            "Rekindle could not prepare its data folder and has to close.\n\n{detail}"
        ))
        .title("Rekindle could not start")
        .kind(MessageDialogKind::Error)
        .show(move |_| handle.exit(1));
}
