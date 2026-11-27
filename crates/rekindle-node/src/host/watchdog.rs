//! systemd integration (`Type=notify`): `READY=1` once the bus accepts
//! connections, the watchdog keep-alive (`WatchdogSec=`), and `STOPPING=1`
//! when shutdown begins. Outside systemd every call is a no-op; systemd runs
//! only on Linux, so on every other OS no service manager is told anything.

use std::time::Duration;

use crate::daemon::heartbeat::Heartbeat;

/// The `sd_notify(3)` states this daemon sends.
#[derive(Clone, Copy)]
enum Signal {
    Ready,
    Stopping,
    Watchdog,
    WatchdogTrigger,
}

#[cfg(target_os = "linux")]
fn send(signal: Signal) -> std::io::Result<()> {
    let state = match signal {
        Signal::Ready => sd_notify::NotifyState::Ready,
        Signal::Stopping => sd_notify::NotifyState::Stopping,
        Signal::Watchdog => sd_notify::NotifyState::Watchdog,
        Signal::WatchdogTrigger => sd_notify::NotifyState::WatchdogTrigger,
    };
    sd_notify::notify(false, &[state])
}

#[cfg(not(target_os = "linux"))]
fn send(_signal: Signal) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "systemd runs only on Linux",
    ))
}

/// Tell systemd the daemon is ready.
pub fn notify_ready() {
    notify(Signal::Ready, "READY=1");
}

/// Tell systemd shutdown has begun (`sd_notify(3)` `STOPPING=1`).
pub fn notify_stopping() {
    notify(Signal::Stopping, "STOPPING=1");
}

fn notify(signal: Signal, name: &str) {
    match send(signal) {
        Ok(()) => tracing::info!("sd_notify: {name} sent"),
        Err(e) => tracing::debug!(error = %e, "sd_notify: {name} failed (not under systemd?)"),
    }
}

/// How often to ping the watchdog: half of `$WATCHDOG_USEC`, as
/// `sd_watchdog_enabled(3)` recommends; `None` when systemd expects no
/// pings.
#[cfg(target_os = "linux")]
#[must_use]
pub fn keepalive_interval() -> Option<Duration> {
    let mut usec = 0;
    sd_notify::watchdog_enabled(false, &mut usec).then(|| Duration::from_micros(usec) / 2)
}

/// No systemd outside Linux, so no watchdog expects pings.
#[cfg(not(target_os = "linux"))]
#[must_use]
pub fn keepalive_interval() -> Option<Duration> {
    None
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
            if let Err(e) = send(Signal::Watchdog) {
                tracing::warn!(error = %e, "sd_notify: watchdog ping failed");
            }
        } else if !reported {
            reported = true;
            tracing::error!("bus subscriber stalled; triggering the watchdog");
            if let Err(e) = send(Signal::WatchdogTrigger) {
                tracing::error!(error = %e, "sd_notify: WATCHDOG=trigger failed");
            }
        }
    }
}
