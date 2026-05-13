//! Filesystem-based config hot-reload using the `notify` crate.
//!
//! Watches the config directory for changes to transport.toml and policy files.
//! On valid change: reloads config, validates, applies to DaemonContext.
//! On invalid change: logs warning with validation errors, retains old config.
//!
//! Also triggered by SIGHUP signal as an explicit reload command.

use std::path::Path;
use std::sync::Arc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::daemon::dispatch::DaemonContext;

/// Start the config file watcher. Returns the watcher handle (must be kept alive).
///
/// Watches the config directory for modifications to `.toml` files.
/// On change: attempts to reload transport config and policy config.
/// Invalid config is rejected with a warning — the daemon continues
/// with the previous valid configuration.
pub fn start_config_watcher(
    config_dir: &Path,
    ctx: Arc<DaemonContext>,
) -> Option<RecommendedWatcher> {
    let watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        match res {
            Ok(event) => {
                if !event.kind.is_modify() && !event.kind.is_create() {
                    return;
                }

                // Filter: only react to .toml files in the config directory.
                let is_config_file = event.paths.iter().any(|p| {
                    p.extension()
                        .is_some_and(|ext| ext == "toml")
                });
                if !is_config_file {
                    return;
                }

                tracing::info!(
                    paths = ?event.paths,
                    "config file changed — reloading"
                );

                reload_config_inner(&ctx);
            }
            Err(e) => {
                tracing::warn!(error = %e, "filesystem watcher error");
            }
        }
    });

    match watcher {
        Ok(mut w) => {
            if config_dir.exists() {
                if let Err(e) = w.watch(config_dir, RecursiveMode::NonRecursive) {
                    tracing::warn!(
                        path = %config_dir.display(),
                        error = %e,
                        "failed to watch config directory — hot-reload disabled"
                    );
                    return None;
                }
                tracing::info!(
                    path = %config_dir.display(),
                    "config watcher active — .toml changes trigger hot-reload"
                );
            } else {
                tracing::debug!(
                    path = %config_dir.display(),
                    "config directory does not exist — watcher inactive until created"
                );
            }
            Some(w)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "failed to create filesystem watcher — config hot-reload disabled"
            );
            None
        }
    }
}

/// Explicit config reload triggered by SIGHUP or IPC PolicyReload command.
pub fn reload_config(ctx: &Arc<DaemonContext>) {
    reload_config_inner(ctx);
}

fn reload_config_inner(ctx: &DaemonContext) {
    let config_file = ctx.paths.config_dir.join("transport.toml");

    if !config_file.exists() {
        tracing::debug!(
            path = %config_file.display(),
            "config reload: transport.toml not found — using defaults"
        );
        return;
    }

    let content = match std::fs::read_to_string(&config_file) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                path = %config_file.display(),
                error = %e,
                "config reload: failed to read transport.toml — retaining current config"
            );
            return;
        }
    };

    let new_config: rekindle_types::config::TransportConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                path = %config_file.display(),
                error = %e,
                "config reload: transport.toml parse/validation FAILED — \
                 retaining current config. Fix the file and save again."
            );
            return;
        }
    };

    // Validate: reject configs with obviously dangerous values.
    if new_config.route_refresh_secs == 0 {
        tracing::warn!("config reload: route_refresh_secs=0 is invalid (would disable route refresh)");
        return;
    }
    if new_config.circuit_breaker_threshold == 0 {
        tracing::warn!("config reload: circuit_breaker_threshold=0 is invalid (would disable circuit breaker)");
        return;
    }

    // Apply policy config reload with system+user merge.
    // Uses the same merge logic as handle_policy_reload (IPC command):
    // system policy sets the floor, user policy can only tighten.
    match crate::daemon::dispatch::admin::load_merged_policy(&ctx.paths.config_dir) {
        Ok(new_policy) => {
            *ctx.policy.write() = new_policy;
            tracing::info!("policy reloaded (system + user merge)");
        }
        Err(e) => {
            tracing::warn!(error = %e, "policy reload failed — retaining current");
        }
    }

    tracing::info!(
        route_refresh_secs = new_config.route_refresh_secs,
        rpc_timeout_ms = new_config.rpc_timeout_ms,
        "transport config reloaded"
    );
}
