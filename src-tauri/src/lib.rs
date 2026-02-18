#![recursion_limit = "512"]

mod channels;
pub mod commands;
pub mod db;
pub mod ipc_client;
pub mod keystore;
mod services;
pub mod state;
mod tray;
mod windows;

use std::sync::Arc;

use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{Code, Modifiers, ShortcutState};

use state::{AppState, SharedState};

#[allow(clippy::too_many_lines)]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    linux_display_setup();

    tracing_subscriber::fmt::init();

    let shared_state: SharedState = Arc::new(AppState::default());
    let state_for_setup = Arc::clone(&shared_state);

    tauri::Builder::default()
        // MUST be first — prevents multiple instances
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("buddy-list") {
                let _ = w.show();
                let _ = w.set_focus();
            } else if let Some(w) = app.get_webview_window("login") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        // NOTE: tauri_plugin_window_state removed — causes infinite windowDidMove
        // event loop on macOS when combined with prevent_exit(). See tauri#11489.
        .plugin(tauri_plugin_store::Builder::new().build())
        // NOTE: tauri-plugin-stronghold removed — we use iota_stronghold directly
        // in keystore.rs with per-identity snapshot files. The plugin was registered
        // but never invoked by the frontend, and its hardcoded production Argon2
        // params conflicted with our debug-mode params.
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(move |app| {
            tray::setup_tray(app)?;

            // Register global keyboard shortcuts (plugin registered here for state access)
            // macOS uses Cmd (Super), Windows/Linux use Ctrl
            let shortcut_state = Arc::clone(&state_for_setup);
            #[cfg(target_os = "macos")]
            let builder = tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts(["super+shift+x", "super+shift+m"])?;
            #[cfg(not(target_os = "macos"))]
            let builder = tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts(["ctrl+shift+x", "ctrl+shift+m"])?;
            app.handle().plugin(
                builder
                    .with_handler(move |app_handle, shortcut, event| {
                        if event.state != ShortcutState::Pressed {
                            return;
                        }

                        // Ctrl+Shift+X / Cmd+Shift+X — toggle buddy list visibility
                        if shortcut.matches(Modifiers::CONTROL | Modifiers::SHIFT, Code::KeyX)
                            || shortcut.matches(Modifiers::SUPER | Modifiers::SHIFT, Code::KeyX)
                        {
                            toggle_buddy_list(app_handle);
                        }

                        // Ctrl+Shift+M / Cmd+Shift+M — toggle voice mute
                        if shortcut.matches(Modifiers::CONTROL | Modifiers::SHIFT, Code::KeyM)
                            || shortcut.matches(Modifiers::SUPER | Modifiers::SHIFT, Code::KeyM)
                        {
                            toggle_mute(app_handle, &shortcut_state);
                        }
                    })
                    .build(),
            )?;

            // Ensure config directory exists on first launch
            let config_dir = app
                .path()
                .app_config_dir()
                .map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&config_dir)
                .map_err(|e| format!("failed to create config dir: {e}"))?;

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

            app.manage(pool);

            // Manage the Stronghold keystore handle (unlocked on login/create_identity)
            app.manage(keystore::new_handle());

            // Note: game detection starts after login (in start_background_services)
            // to avoid burning CPU before user is authenticated.

            // Start the Veilid node at app startup — the node lives for the
            // entire app lifetime. User login/logout is independent of node lifecycle.
            let state_for_veilid = Arc::clone(&state_for_setup);
            let app_handle_clone = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match services::veilid_service::initialize_node(&app_handle_clone, &state_for_veilid).await {
                    Ok(update_rx) => {
                        // Create shutdown channel for the dispatch loop
                        let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel(1);
                        *state_for_veilid.shutdown_tx.write() = Some(shutdown_tx);

                        // Start dispatch loop (runs until app exit)
                        let dispatch_state = Arc::clone(&state_for_veilid);
                        let dispatch_handle = tokio::spawn(services::veilid_service::start_dispatch_loop(
                            app_handle_clone,
                            dispatch_state,
                            update_rx,
                            shutdown_rx,
                        ));
                        *state_for_veilid.dispatch_loop_handle.write() = Some(dispatch_handle);
                    }
                    Err(e) => tracing::error!(error = %e, "failed to start Veilid node at app startup"),
                }
            });

            // Emit startup notification
            let notification = channels::NotificationEvent::SystemAlert {
                title: "Rekindle".to_string(),
                body: "Application started successfully".to_string(),
            };
            let _ = app.emit("notification-event", &notification);

            tracing::info!("Rekindle started");
            Ok(())
        })
        .manage(shared_state)
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let label = window.label();
                if label == "buddy-list" {
                    // Hide buddy list to tray instead of closing
                    let _ = window.hide();
                    api.prevent_close();
                } else if label == "login" {
                    // If login is closed and no buddy-list visible, exit the app
                    let has_buddy = window
                        .app_handle()
                        .get_webview_window("buddy-list")
                        .and_then(|w| w.is_visible().ok())
                        .unwrap_or(false);
                    if !has_buddy {
                        window.app_handle().exit(0);
                    }
                }
                // Other windows (chat, settings, etc.) close normally
            }
        })
        .invoke_handler(tauri::generate_handler![
            // auth
            commands::auth::create_identity,
            commands::auth::login,
            commands::auth::get_identity,
            commands::auth::logout,
            commands::auth::list_identities,
            commands::auth::delete_identity,
            // chat
            commands::chat::prepare_chat_session,
            commands::chat::send_message,
            commands::chat::send_typing,
            commands::chat::get_message_history,
            commands::chat::mark_read,
            // friends
            commands::friends::add_friend,
            commands::friends::remove_friend,
            commands::friends::accept_request,
            commands::friends::get_friends,
            commands::friends::reject_request,
            commands::friends::get_pending_requests,
            commands::friends::create_friend_group,
            commands::friends::rename_friend_group,
            commands::friends::move_friend_to_group,
            commands::friends::generate_invite,
            commands::friends::add_friend_from_invite,
            commands::friends::block_friend,
            commands::friends::emit_friends_presence,
            // community
            commands::community::create_community,
            commands::community::join_community,
            commands::community::create_channel,
            commands::community::send_channel_message,
            commands::community::get_channel_messages,
            commands::community::get_communities,
            commands::community::get_community_details,
            commands::community::get_community_members,
            commands::community::remove_community_member,
            commands::community::get_roles,
            commands::community::create_role,
            commands::community::edit_role,
            commands::community::delete_role,
            commands::community::assign_role,
            commands::community::unassign_role,
            commands::community::timeout_member,
            commands::community::remove_timeout,
            commands::community::set_channel_overwrite,
            commands::community::delete_channel_overwrite,
            commands::community::leave_community,
            commands::community::delete_channel,
            commands::community::rename_channel,
            commands::community::update_community_info,
            commands::community::ban_member,
            commands::community::unban_member,
            commands::community::get_ban_list,
            commands::community::rotate_mek,
            // voice
            commands::voice::join_voice_channel,
            commands::voice::leave_voice,
            commands::voice::set_mute,
            commands::voice::set_deafen,
            commands::voice::list_audio_devices,
            commands::voice::set_audio_devices,
            // status
            commands::status::set_status,
            commands::status::set_nickname,
            commands::status::set_avatar,
            commands::status::get_avatar,
            commands::status::set_status_message,
            // game
            commands::game::get_game_status,
            // settings
            commands::settings::get_preferences,
            commands::settings::set_preferences,
            commands::settings::check_for_updates,
            // windows
            commands::window::show_buddy_list,
            commands::window::open_chat_window,
            commands::window::open_settings_window,
            commands::window::open_community_window,
            commands::window::open_profile_window,
            commands::window::get_network_status,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Rekindle")
        .run(|app_handle, event| match &event {
            tauri::RunEvent::ExitRequested { code, api, .. } => {
                // code: None  = all windows closed (keep alive for tray icon)
                // code: Some  = programmatic exit via app.exit() (let it proceed)
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            tauri::RunEvent::Exit => {
                // app.exit(0) was called (from tray quit or system shutdown).
                // Run graceful shutdown with a timeout to prevent hanging.
                tracing::info!("RunEvent::Exit fired — starting graceful shutdown");
                let state: tauri::State<'_, SharedState> = app_handle.state();
                let state = state.inner().clone();
                tauri::async_runtime::block_on(async move {
                    let shutdown = graceful_shutdown(&state);
                    if tokio::time::timeout(std::time::Duration::from_secs(5), shutdown)
                        .await
                        .is_err()
                    {
                        tracing::warn!("graceful shutdown timed out after 5s — forcing exit");
                    }
                });
                // Force process termination. On macOS the event loop may not
                // exit cleanly when a system tray icon is active, leaving the
                // process alive after RunEvent::Exit returns.
                std::process::exit(0);
            }
            _ => {}
        });
}

/// Shut down all background services before the process exits.
///
/// First cleans up user-specific state (DHT records, routes), then sends
/// shutdown signals to the dispatch loop, and finally shuts down the
/// Veilid node itself.
#[allow(clippy::too_many_lines)]
async fn graceful_shutdown(state: &SharedState) {
    tracing::info!("graceful shutdown: stopping background services");

    // 1. Send graceful shutdown signals to all services FIRST.
    //    This gives them a chance to finish their current operation before
    //    logout_cleanup aborts any remaining handles.

    // Signal sync service shutdown
    let sync_tx = state.sync_shutdown_tx.read().clone();
    if let Some(tx) = sync_tx {
        let _ = tx.send(()).await;
    }

    // Signal game detection shutdown
    let game_tx = state.game_detector.lock().as_ref().map(|h| h.shutdown_tx.clone());
    if let Some(tx) = game_tx {
        let _ = tx.send(()).await;
    }

    // Signal route refresh loop shutdown
    let route_refresh_tx = state.route_refresh_shutdown_tx.write().take();
    if let Some(tx) = route_refresh_tx {
        let _ = tx.send(()).await;
    }

    // Signal idle service shutdown
    let idle_tx = state.idle_shutdown_tx.write().take();
    if let Some(tx) = idle_tx {
        let _ = tx.send(()).await;
    }

    // Signal heartbeat shutdown
    let heartbeat_tx = state.heartbeat_shutdown_tx.write().take();
    if let Some(tx) = heartbeat_tx {
        let _ = tx.send(()).await;
    }

    // Signal dispatch loop shutdown
    let shutdown_tx = state.shutdown_tx.read().clone();
    if let Some(tx) = shutdown_tx {
        let _ = tx.send(()).await;
    }

    // 2. Shut down voice engine (signal loops, await, then stop devices)
    {
        let (send_tx, send_h, recv_tx, recv_h, monitor_tx, monitor_h) = {
            let mut ve = state.voice_engine.lock();
            if let Some(ref mut handle) = *ve {
                (
                    handle.send_loop_shutdown.take(),
                    handle.send_loop_handle.take(),
                    handle.recv_loop_shutdown.take(),
                    handle.recv_loop_handle.take(),
                    handle.device_monitor_shutdown.take(),
                    handle.device_monitor_handle.take(),
                )
            } else {
                (None, None, None, None, None, None)
            }
        };
        if let Some(tx) = send_tx { let _ = tx.send(()).await; }
        if let Some(tx) = recv_tx { let _ = tx.send(()).await; }
        if let Some(tx) = monitor_tx { let _ = tx.send(()).await; }
        if let Some(h) = send_h { let _ = h.await; }
        if let Some(h) = recv_h { let _ = h.await; }
        if let Some(h) = monitor_h { let _ = h.await; }
        {
            let mut ve = state.voice_engine.lock();
            if let Some(ref mut handle) = *ve {
                handle.engine.stop_capture();
                handle.engine.stop_playback();
            }
            *ve = None;
        }
        *state.voice_packet_tx.write() = None;
    }

    // 3. Shut down server health check loop
    {
        let tx = state.server_health_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }

    // 4. Shut down community server process (if running)
    {
        // Try graceful shutdown via IPC first
        let socket_path = crate::ipc_client::default_socket_path();
        if socket_path.exists() {
            if let Err(e) = crate::ipc_client::shutdown_server_blocking(&socket_path) {
                tracing::debug!(error = %e, "IPC shutdown failed — will kill process");
            }
        }

        let mut proc = state.server_process.lock();
        if let Some(ref mut child) = *proc {
            tracing::info!("stopping community server process");
            // Give the server a moment to exit gracefully after IPC shutdown
            if !matches!(child.try_wait(), Ok(Some(_))) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        *proc = None;
    }

    // 5. Await the dispatch loop handle (it should have exited after the shutdown signal)
    {
        let dispatch_handle = state.dispatch_loop_handle.write().take();
        if let Some(h) = dispatch_handle {
            let _ = h.await;
        }
    }

    // 6. Publish Offline to DHT before cleanup closes records
    {
        let current_status = state.identity.read().as_ref().map(|id| id.status);
        if current_status != Some(state::UserStatus::Offline) {
            if let Err(e) =
                services::presence_service::publish_status(state, state::UserStatus::Offline).await
            {
                tracing::warn!(error = %e, "failed to publish offline on shutdown");
            }
        }
    }

    // 7. Now clean up user-specific DHT state (close records, release route,
    //    abort remaining background handles).
    //    Pass None for app_handle — the app is exiting, no UI to update.
    services::veilid_service::logout_cleanup(None, state).await;

    // Clear community state
    state.mek_cache.lock().clear();
    state.community_routes.write().clear();

    // 8. Shut down the Veilid node (only on app exit)
    services::veilid_service::shutdown_app(state).await;

    tracing::info!("graceful shutdown complete");
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

/// Toggle the buddy list window visibility (Ctrl+Shift+X / Cmd+Shift+X).
fn toggle_buddy_list(app_handle: &tauri::AppHandle) {
    if let Some(window) = app_handle.get_webview_window("buddy-list") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
            tracing::debug!("buddy list hidden via global shortcut");
        } else {
            let _ = window.show();
            let _ = window.set_focus();
            tracing::debug!("buddy list shown via global shortcut");
        }
    }
}

/// Toggle voice mute state (Ctrl+Shift+M / Cmd+Shift+M).
///
/// Flips the mute flag on the voice engine and emits a `VoiceEvent::UserMuted`
/// event so the frontend stays in sync.
fn toggle_mute(app_handle: &tauri::AppHandle, state: &SharedState) {
    // Read identity key first to avoid holding two locks simultaneously
    let public_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();

    let mut ve = state.voice_engine.lock();
    if let Some(ref mut handle) = *ve {
        let new_muted = !handle.engine.is_muted;
        handle.engine.set_muted(new_muted);
        handle.muted_flag.store(new_muted, std::sync::atomic::Ordering::Relaxed);

        let event = channels::VoiceEvent::UserMuted {
            public_key,
            muted: new_muted,
        };
        let _ = app_handle.emit("voice-event", &event);

        tracing::debug!(muted = new_muted, "voice mute toggled via global shortcut");
    }
}

/// Linux WebKitGTK environment setup — must run before tauri::Builder.
///
/// 1. Wayland discovery — tmux/SSH/TTY sessions don't inherit WAYLAND_DISPLAY
///    from the compositor. Scan XDG_RUNTIME_DIR for the socket.
///
/// 2. NVIDIA + WebKitGTK workarounds — proprietary drivers have known issues
///    with WebKitGTK's DMABuf renderer and explicit sync on all distros.
///
/// All vars are skipped if already set, so users can always override.
///
/// See: https://github.com/tauri-apps/tauri/issues/9394
#[cfg(target_os = "linux")]
fn linux_display_setup() {
    use std::path::Path;

    // Wayland display discovery
    if std::env::var("WAYLAND_DISPLAY").unwrap_or_default().is_empty() {
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            if let Ok(entries) = std::fs::read_dir(&runtime_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with("wayland-") && !name.ends_with(".lock") {
                        std::env::set_var("WAYLAND_DISPLAY", &*name);
                        break;
                    }
                }
            }
        }
    }

    // NVIDIA workarounds
    if Path::new("/proc/driver/nvidia/version").exists() {
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
        if std::env::var("__NV_DISABLE_EXPLICIT_SYNC").is_err() {
            std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
        }
    }
}
