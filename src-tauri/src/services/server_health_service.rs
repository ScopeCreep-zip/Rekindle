use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::ipc_client;
use crate::state::SharedState;

/// Interval between health checks.
const CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Number of consecutive failures before attempting a restart.
const FAILURE_THRESHOLD: u32 = 2;

/// Minimum time between restart attempts to prevent thrashing.
const MIN_RESTART_INTERVAL: Duration = Duration::from_secs(120);

/// Server health check loop: periodically pings the rekindle-server process
/// and restarts it if unresponsive.
///
/// Runs as a background task spawned after `maybe_spawn_server`. Shuts down
/// when the `shutdown_rx` channel fires or when the server process is cleared
/// from `AppState` (e.g. on logout).
pub async fn server_health_loop(
    state: SharedState,
    app_handle: tauri::AppHandle,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    let mut interval = tokio::time::interval(CHECK_INTERVAL);
    let mut consecutive_failures: u32 = 0;
    let mut last_restart: Option<Instant> = None;

    // Skip the first tick (fires immediately) — give the server time to start
    interval.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // If server process is not set, we shouldn't be running
                let has_server = state.server_process.lock().is_some();
                if !has_server {
                    tracing::debug!("server process not set — health check stopping");
                    break;
                }

                let socket_path = ipc_client::default_socket_path();
                let sp = socket_path.clone();
                let status = tokio::task::spawn_blocking(move || {
                    ipc_client::get_status_blocking(&sp)
                })
                .await;

                match status {
                    Ok(Ok((uptime, communities, attached))) => {
                        if consecutive_failures > 0 {
                            tracing::info!(
                                uptime_secs = uptime,
                                communities,
                                veilid_attached = attached,
                                "server health check recovered after {} failures",
                                consecutive_failures
                            );
                        }
                        consecutive_failures = 0;
                    }
                    Ok(Err(e)) => {
                        consecutive_failures += 1;
                        tracing::warn!(
                            error = %e,
                            consecutive = consecutive_failures,
                            "server health check failed"
                        );
                    }
                    Err(e) => {
                        consecutive_failures += 1;
                        tracing::warn!(
                            error = %e,
                            consecutive = consecutive_failures,
                            "server health check task panicked"
                        );
                    }
                }

                if consecutive_failures >= FAILURE_THRESHOLD {
                    let should_restart = match last_restart {
                        Some(t) => t.elapsed() >= MIN_RESTART_INTERVAL,
                        None => true,
                    };

                    if should_restart {
                        tracing::warn!(
                            failures = consecutive_failures,
                            "rekindle-server unresponsive — attempting restart"
                        );
                        restart_server(&state, &app_handle);
                        last_restart = Some(Instant::now());
                        consecutive_failures = 0;
                    } else {
                        tracing::debug!(
                            "skipping restart — last restart too recent ({:.0}s ago)",
                            last_restart.unwrap().elapsed().as_secs_f64()
                        );
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::debug!("server health check loop shutting down");
                break;
            }
        }
    }
}

/// Kill the existing server process and re-spawn it.
fn restart_server(state: &SharedState, app_handle: &tauri::AppHandle) {
    // Kill old process
    {
        let mut proc = state.server_process.lock();
        if let Some(ref mut child) = *proc {
            let pid = child.id();
            tracing::info!(pid, "killing unresponsive server process");
            let _ = child.kill();
            let _ = child.wait();
        }
        *proc = None;
    }

    // Clean up stale socket file
    let socket_path = ipc_client::default_socket_path();
    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }

    // Re-spawn via the existing function
    crate::commands::auth::maybe_spawn_server(app_handle, state);
}
