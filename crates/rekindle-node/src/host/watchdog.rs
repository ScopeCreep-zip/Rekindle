//! systemd integration (`Type=notify`): `READY=1` once the bus accepts
//! connections, the watchdog keep-alive (`WatchdogSec=`), and `STOPPING=1`
//! when shutdown begins. Outside systemd every call is a no-op.

use std::time::Duration;

use crate::daemon::heartbeat::Heartbeat;

/// Tell systemd the daemon is ready.
pub fn notify_ready() {
    notify(sd_notify::NotifyState::Ready, "READY=1");
}

/// Tell systemd shutdown has begun (`sd_notify(3)` `STOPPING=1`).
pub fn notify_stopping() {
    notify(sd_notify::NotifyState::Stopping, "STOPPING=1");
}

fn notify(state: sd_notify::NotifyState<'_>, name: &str) {
    match sd_notify::notify(false, &[state]) {
        Ok(()) => tracing::info!("sd_notify: {name} sent"),
        Err(e) => tracing::debug!(error = %e, "sd_notify: {name} failed (not under systemd?)"),
    }
}

/// How often to ping the watchdog: half of `$WATCHDOG_USEC`, as
/// `sd_watchdog_enabled(3)` recommends; `None` when systemd expects no
/// pings.
#[must_use]
pub fn keepalive_interval() -> Option<Duration> {
    let mut usec = 0;
    sd_notify::watchdog_enabled(false, &mut usec).then(|| Duration::from_micros(usec) / 2)
}

/// Ping the watchdog every `interval` while the subscriber's heartbeat is
/// fresh. A stale heartbeat is reported once with `WATCHDOG=trigger`
/// (`sd_notify(3)`: "the service detected an internal error that should be
/// handled by the configured watchdog options"). Never completes; without
/// a watchdog it only waits.
pub async fn keep_alive(interval: Option<Duration>, heartbeat: &Heartbeat) {
    let Some(interval) = interval else {
        return std::future::pending().await;
    };
    let mut ticks = tokio::time::interval(interval);
    let mut reported = false;
    loop {
        ticks.tick().await;
        if heartbeat.is_fresh() {
            reported = false;
            if let Err(e) = sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]) {
                tracing::warn!(error = %e, "sd_notify: watchdog ping failed");
            }
        } else if !reported {
            reported = true;
            tracing::error!("bus subscriber stalled; triggering the watchdog");
            if let Err(e) = sd_notify::notify(false, &[sd_notify::NotifyState::WatchdogTrigger]) {
                tracing::error!(error = %e, "sd_notify: WATCHDOG=trigger failed");
            }
        }
    }
}
