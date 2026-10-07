//! Wayland `ext-idle-notify-v1` idle monitor for Linux.
//!
//! Connects to the compositor as a persistent Wayland client and subscribes to
//! idle/resumed events with a 1-second timeout. This works on COSMIC, Sway, and
//! any other compositor implementing the `ext-idle-notify-v1` protocol — unlike
//! `xprintidle` (X11-only) or Mutter D-Bus (GNOME-only).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1, ext_idle_notifier_v1,
};

/// Global singleton — set once when the Wayland monitor thread starts.
/// Persists for process lifetime (Wayland connection is long-lived).
static WAYLAND_IDLE: OnceLock<Arc<WaylandIdleState>> = OnceLock::new();

/// Sentinel value for "user is currently active" (no idle timestamp).
const ACTIVE_SENTINEL: u64 = u64::MAX;

/// Shared idle state updated by the Wayland event thread, read by the
/// polling idle service.
pub struct WaylandIdleState {
    /// Monotonic offset (seconds since `start`) when user became idle.
    /// `u64::MAX` means user is currently active.
    idle_since: AtomicU64,
    /// Process-local monotonic reference point.
    start: Instant,
}

impl WaylandIdleState {
    fn new() -> Self {
        Self {
            idle_since: AtomicU64::new(ACTIVE_SENTINEL),
            start: Instant::now(),
        }
    }

    fn mark_idle(&self) {
        let now = self.start.elapsed().as_secs();
        self.idle_since.store(now, Ordering::Relaxed);
    }

    fn mark_active(&self) {
        self.idle_since.store(ACTIVE_SENTINEL, Ordering::Relaxed);
    }

    /// Returns idle duration in seconds, or `0` if active.
    /// The 1-second notification timeout is added to the elapsed idle time.
    pub fn get_idle_seconds(&self) -> u64 {
        let since = self.idle_since.load(Ordering::Relaxed);
        if since == ACTIVE_SENTINEL {
            return 0;
        }
        let now = self.start.elapsed().as_secs();
        // Add 1s for the notification timeout (user was idle 1s before we got notified)
        now.saturating_sub(since) + 1
    }
}

/// Wayland client state for event dispatching.
struct WlState {
    idle: Arc<WaylandIdleState>,
}

// Registry — required by `registry_queue_init`, no events we need
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WlState {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

// Notifier global emits no client-side events
delegate_noop!(WlState: ext_idle_notifier_v1::ExtIdleNotifierV1);

// Seat events not needed here
delegate_noop!(WlState: ignore wl_seat::WlSeat);

// Idle/resumed event handler
impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for WlState {
    fn event(
        state: &mut Self,
        _proxy: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => {
                tracing::debug!("wayland idle monitor: user idle");
                state.idle.mark_idle();
            }
            ext_idle_notification_v1::Event::Resumed => {
                tracing::debug!("wayland idle monitor: user resumed");
                state.idle.mark_active();
            }
            _ => {}
        }
    }
}

/// Spawn a background thread that connects to the Wayland compositor and
/// listens for idle/resumed events. The thread runs for the process lifetime.
fn start_wayland_monitor(idle: Arc<WaylandIdleState>) {
    std::thread::Builder::new()
        .name("wayland-idle-monitor".into())
        .spawn(move || {
            let conn = match Connection::connect_to_env() {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!("wayland idle monitor: failed to connect: {e}");
                    return;
                }
            };

            let (globals, mut event_queue) = match registry_queue_init::<WlState>(&conn) {
                Ok(g) => g,
                Err(e) => {
                    tracing::debug!("wayland idle monitor: registry init failed: {e}");
                    return;
                }
            };

            let qh = event_queue.handle();

            let notifier =
                match globals.bind::<ext_idle_notifier_v1::ExtIdleNotifierV1, _, _>(&qh, 1..=1, ())
                {
                    Ok(n) => n,
                    Err(e) => {
                        tracing::debug!(
                            "wayland idle monitor: ext-idle-notify-v1 not supported: {e}"
                        );
                        return;
                    }
                };

            let seat = match globals.bind::<wl_seat::WlSeat, _, _>(&qh, 1..=1, ()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!("wayland idle monitor: no wl_seat: {e}");
                    return;
                }
            };

            // 1-second timeout — we get notified 1s after last input, then
            // compute actual idle duration from the timestamp.
            let _notification = notifier.get_idle_notification(1000, &seat, &qh, ());

            let mut state = WlState { idle: idle.clone() };

            tracing::info!("wayland idle monitor: started (ext-idle-notify-v1)");

            loop {
                if let Err(e) = event_queue.blocking_dispatch(&mut state) {
                    tracing::debug!("wayland idle monitor: dispatch error: {e}");
                    break;
                }
            }

            tracing::debug!("wayland idle monitor: exiting");
        })
        .ok();
}

/// Try to initialize the Wayland idle monitor. Idempotent — only the
/// first call (per process) that observes `WAYLAND_DISPLAY`/
/// `WAYLAND_SOCKET` and wins the `OnceLock` race actually spawns the
/// monitor thread; every later call is a cheap no-op.
pub fn try_init() {
    let on_wayland =
        std::env::var("WAYLAND_DISPLAY").is_ok() || std::env::var("WAYLAND_SOCKET").is_ok();
    if on_wayland && WAYLAND_IDLE.get().is_none() {
        let state = Arc::new(WaylandIdleState::new());
        if WAYLAND_IDLE.set(state.clone()).is_ok() {
            start_wayland_monitor(state);
        }
    }
}

/// Query idle seconds from the Wayland monitor, if running.
pub fn get_idle_seconds() -> Option<u64> {
    Some(WAYLAND_IDLE.get()?.get_idle_seconds())
}
