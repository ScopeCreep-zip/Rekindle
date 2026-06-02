//! App setup: runs once during `tauri::Builder::setup`, wiring state, storage,
//! background services, and the Veilid node.

use std::sync::Arc;

use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

use crate::state::SharedState;
use crate::{
    channels, db, event_dispatch, friend_store_sqlite, services, shortcuts, tray,
};

/// Run all one-time app setup. Invoked from the `tauri::Builder::setup` closure.
pub fn run(app: &tauri::App, state: &SharedState) -> Result<(), Box<dyn std::error::Error>> {
    // Store app handle in AppState so background services can emit events
    *state.app_handle.write() = Some(app.handle().clone());
    services::community::start_write_retry_worker(Arc::clone(state));

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

    // Ensure config directory exists on first launch
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("failed to create config dir: {e}"))?;

    // Phase 2 — detect leftover Stronghold-era identity files. We DO
    // NOT auto-delete (the user can remove them manually once they've
    // confirmed nothing else needs them); we just surface a one-shot
    // toast on this boot so the user knows their old identities are
    // gone and must be recreated. Plan reference:
    // `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md` § Phase 2.
    let legacy_strongholds: Vec<std::path::PathBuf> = std::fs::read_dir(&config_dir)
        .ok()
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with(".stronghold"))
                })
                .collect()
        })
        .unwrap_or_default();
    if !legacy_strongholds.is_empty() {
        tracing::warn!(
            count = legacy_strongholds.len(),
            "legacy .stronghold files detected — identity format upgraded in Phase 2"
        );
        let body = format!(
            "Found {} identity file{} from a previous Rekindle version. \
             Please recreate your identity. Old files were not deleted — \
             you can remove them manually from {}.",
            legacy_strongholds.len(),
            if legacy_strongholds.len() == 1 {
                ""
            } else {
                "s"
            },
            config_dir.display(),
        );
        let _ = app.handle().emit(
            "notification-event",
            &channels::NotificationEvent::SystemAlert {
                title: "Identity format upgraded".to_string(),
                body,
            },
        );
    }

    // Resolve the Lost Cargo cache root (per-community sub-dirs are
    // created on first use). Stays None if the platform doesn't expose
    // an app data dir — file uploads will fail clearly in that case.
    if let Ok(data_dir) = app.path().app_data_dir() {
        let cache_root = data_dir.join("file_cache");
        if let Err(e) = std::fs::create_dir_all(&cache_root) {
            tracing::warn!(error = %e, path = %cache_root.display(), "failed to create file_cache dir");
        } else {
            *state.file_cache_root.write() = Some(cache_root);
        }
    }

    // Initialize SQLite database pool
    let db_path = config_dir.join("rekindle.db");
    let db_path_str = db_path.to_string_lossy().to_string();
    let db::DbOpenResult { pool, schema_reset } = db::create_pool(&db_path_str)?;

    // When the schema version changes, all SQLite tables are dropped and
    // recreated.  Stronghold files and Veilid's local storage must also
    // be wiped so there's no orphaned state (stale DHT records, old
    // private keys whose identity rows no longer exist).
    if schema_reset {
        wipe_dependent_storage(&config_dir, app);
    }

    // Phase 2 Track A — wire SqliteFriendStore into AppState BEFORE
    // the Veilid dispatch loop spawns. This eliminates the
    // dispatch-before-hydration race that caused
    // "not friends even though we are" AEAD failures.
    let pool_for_friend_store = Arc::new(pool.clone());
    let friend_store: Arc<dyn rekindle_transport::FriendStore> = Arc::new(
        friend_store_sqlite::SqliteFriendStore::new(pool_for_friend_store),
    );
    *state.friend_store.write() = Some(friend_store);

    app.manage(pool);

    // Manage the Stronghold keystore handle (unlocked on login/create_identity).
    // AppState owns the same `Arc<Mutex<Option<StrongholdKeystore>>>` so
    // services with `&AppState` (audit tail-anchor persistence in
    // particular) can reach the vault without threading Tauri State
    // through every callsite.
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
                        // Phase 5 — payload shape per plan: { state, at_ms }.
                        // Frontend renders state badges live + timestamps for
                        // diagnostic "last transition" displays.
                        let payload = serde_json::json!({
                            "state": next,
                            "at_ms": rekindle_utils::timestamp_ms_i64(),
                        });
                        event_dispatch::emit_live(&app_for_lifecycle, "lifecycle-event", &payload);
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
            }
        }
    });

    // Phase 23.A — spawn the single event-dispatch loop. Every
    // subsequent `emit_live` / `emit_journaled` call pushes
    // through this loop's mpsc; this is the only place
    // `app.emit()` runs inside the app.
    event_dispatch::spawn_dispatch_loop(app.handle().clone(), &state.event_dispatch);

    // Emit startup notification
    let notification = channels::NotificationEvent::SystemAlert {
        title: "Rekindle".to_string(),
        body: "Application started successfully".to_string(),
    };
    event_dispatch::emit_now(state, "notification-event", &notification);

    tracing::info!("Rekindle started");
    Ok(())
}

/// Wipe Stronghold snapshot files and Veilid local storage that are now
/// orphaned after a schema reset.  Without this, old `.stronghold` files
/// and cached DHT records would cause "wrong password" and "record already
/// exists" errors on re-login.
fn wipe_dependent_storage(config_dir: &std::path::Path, app: &tauri::App) {
    // 1. Remove all .stronghold files and orphaned temp files in config dir.
    //    Temp files are created by Stronghold's atomic write (encrypt_file)
    //    and have the pattern `{name}.stronghold.{hex_salt}`.
    if let Ok(entries) = std::fs::read_dir(config_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".stronghold") || name.contains(".stronghold.") {
                if let Err(e) = std::fs::remove_file(&path) {
                    tracing::warn!(path = %path.display(), error = %e, "failed to remove orphaned stronghold file");
                } else {
                    tracing::info!(path = %path.display(), "removed orphaned stronghold file");
                }
            }
        }
    }

    // 2. Remove Veilid local storage directory (DHT record cache, table store, etc.)
    if let Ok(data_dir) = app.path().app_data_dir() {
        let veilid_dir = data_dir.join("veilid");
        if veilid_dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&veilid_dir) {
                tracing::warn!(path = %veilid_dir.display(), error = %e, "failed to remove veilid storage");
            } else {
                tracing::info!(path = %veilid_dir.display(), "removed orphaned veilid storage");
            }
        }
    }

    // NOTE: Veilid's protected_store (macOS Keychain) is NOT wiped here.
    // The device encryption key is the node's persistent identity on the
    // Veilid network and must survive schema resets.
}
