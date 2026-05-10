//! systemd Type=notify integration.
//!
//! Provides READY=1 notification, watchdog keepalive, and STATUS= updates
//! visible in `systemctl status rekindle-node`.
//!
//! All functions are no-ops when not running under systemd (NOTIFY_SOCKET unset).

/// Notify systemd that the daemon is ready (READY=1).
///
/// `unset_environment` is false to preserve NOTIFY_SOCKET for subsequent
/// watchdog keepalive pings and status updates.
pub fn notify_ready() {
    match sd_notify::notify(false, &[sd_notify::NotifyState::Ready]) {
        Ok(()) => tracing::info!("sd_notify: READY=1"),
        Err(e) => tracing::debug!(error = %e, "sd_notify: READY=1 failed (not under systemd?)"),
    }
}

/// Send a watchdog keepalive to systemd.
///
/// Must be sent at intervals less than WatchdogSec configured in the unit file.
/// The daemon's watchdog interval (15s) is well under the recommended 30s
/// WatchdogSec to allow for timer jitter.
pub fn notify_watchdog() {
    if let Err(e) = sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]) {
        tracing::trace!(error = %e, "sd_notify: watchdog ping failed");
    }
}

/// Update the daemon's status string visible in `systemctl status`.
///
/// Used to surface the current daemon state (locked, operational, degraded)
/// and brief context (peer count, watch count) to operators using systemctl.
pub fn notify_status(status: &str) {
    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Status(status)]);
}

/// Notify systemd that the daemon is stopping.
///
/// Sent at the beginning of the shutdown sequence. systemd uses this to
/// distinguish between a clean shutdown (STOPPING=1 → exit 0) and a crash
/// (no STOPPING, unexpected exit).
pub fn notify_stopping() {
    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Stopping]);
}
