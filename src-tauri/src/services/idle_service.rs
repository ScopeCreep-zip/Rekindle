use std::sync::Arc;

use tauri::Emitter;
use tauri_plugin_store::StoreExt;
use tokio::sync::mpsc;

use crate::state::{AppState, UserStatus};

/// Wayland `ext-idle-notify-v1` idle monitor for Linux.
///
/// Connects to the compositor as a persistent Wayland client and subscribes to
/// idle/resumed events with a 1-second timeout. This works on COSMIC, Sway, and
/// any other compositor implementing the `ext-idle-notify-v1` protocol — unlike
/// `xprintidle` (X11-only) or Mutter D-Bus (GNOME-only).
#[cfg(target_os = "linux")]
mod wayland_idle {
    use std::sync::atomic::{AtomicI64, Ordering};
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

    /// Shared idle state updated by the Wayland event thread, read by the
    /// polling idle service.
    pub struct WaylandIdleState {
        /// Monotonic offset (seconds since `start`) when user became idle.
        /// `-1` means user is currently active.
        idle_since: AtomicI64,
        /// Process-local monotonic reference point.
        start: Instant,
    }

    impl WaylandIdleState {
        fn new() -> Self {
            Self {
                idle_since: AtomicI64::new(-1),
                start: Instant::now(),
            }
        }

        fn mark_idle(&self) {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_possible_wrap,
                reason = "Instant elapsed as i64 seconds won't overflow for process lifetime"
            )]
            let now = self.start.elapsed().as_secs() as i64;
            self.idle_since.store(now, Ordering::Relaxed);
        }

        fn mark_active(&self) {
            self.idle_since.store(-1, Ordering::Relaxed);
        }

        /// Returns idle duration in seconds, or `Some(0)` if active.
        /// The 1-second notification timeout is added to the elapsed idle time.
        pub fn get_idle_seconds(&self) -> Option<u64> {
            let since = self.idle_since.load(Ordering::Relaxed);
            if since < 0 {
                return Some(0);
            }
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_possible_wrap,
                reason = "Instant elapsed as i64 seconds won't overflow for process lifetime"
            )]
            let now = self.start.elapsed().as_secs() as i64;
            // Add 1s for the notification timeout (user was idle 1s before we got notified)
            #[allow(
                clippy::cast_sign_loss,
                reason = "now >= since is guaranteed (monotonic clock); result is non-negative"
            )]
            Some((now - since) as u64 + 1)
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

                let notifier = match globals
                    .bind::<ext_idle_notifier_v1::ExtIdleNotifierV1, _, _>(&qh, 1..=1, ())
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

                let mut state = WlState {
                    idle: idle.clone(),
                };

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

    /// Try to initialize the Wayland idle monitor (called once at service start).
    pub fn try_init() {
        if std::env::var("WAYLAND_DISPLAY").is_ok()
            || std::env::var("WAYLAND_SOCKET").is_ok()
        {
            if WAYLAND_IDLE.get().is_none() {
                let state = Arc::new(WaylandIdleState::new());
                if WAYLAND_IDLE.set(state.clone()).is_ok() {
                    start_wayland_monitor(state);
                }
            }
        }
    }

    /// Query idle seconds from the Wayland monitor, if running.
    pub fn get_idle_seconds() -> Option<u64> {
        WAYLAND_IDLE.get()?.get_idle_seconds()
    }
}

/// Get system idle time in seconds (platform-specific).
fn get_idle_seconds() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        macos_idle_seconds()
    }
    #[cfg(target_os = "windows")]
    {
        windows_idle_seconds()
    }
    #[cfg(target_os = "linux")]
    {
        linux_idle_seconds()
    }
}

/// macOS: Use `CGEventSourceSecondsSinceLastEventType` from CoreGraphics.
///
/// This is the same approach used by Chromium (and thus Discord, Slack, Signal
/// Desktop, and every Electron app) for idle detection since 2012. It returns
/// seconds since the last user input event (mouse, keyboard, trackpad).
///
/// The previous `ioreg -c IOHIDSystem` approach fails to parse `HIDIdleTime`
/// on Darwin 25.x (macOS Tahoe). `CGEventSource` is a single FFI call with
/// no subprocess spawning or stdout parsing.
///
/// Reference: Chromium `ui/base/idle/idle_mac.mm`
#[cfg(target_os = "macos")]
fn macos_idle_seconds() -> Option<u64> {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(
            source_state_id: i32,
            event_type: u32,
        ) -> f64;
    }
    // kCGEventSourceStateCombinedSessionState = 0
    // kCGAnyInputEventType = 0xFFFFFFFF (u32::MAX)
    let secs = unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) };
    // Negative means error; NaN/Inf are also invalid
    if secs.is_finite() && secs >= 0.0 {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "secs is validated non-negative and finite; truncation to u64 is intentional"
        )]
        Some(secs as u64)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn windows_idle_seconds() -> Option<u64> {
    #[repr(C)]
    struct LastInputInfo {
        cb_size: u32,
        dw_time: u32,
    }
    extern "system" {
        fn GetLastInputInfo(plii: *mut LastInputInfo) -> i32;
        fn GetTickCount() -> u32;
    }
    let mut lii = LastInputInfo {
        cb_size: 8,
        dw_time: 0,
    };
    if unsafe { GetLastInputInfo(&mut lii) } != 0 {
        let idle_ms = unsafe { GetTickCount() }.wrapping_sub(lii.dw_time);
        Some(u64::from(idle_ms) / 1000)
    } else {
        None
    }
}

/// Linux: try multiple idle detection methods for X11 and Wayland coverage.
///
/// 1. Wayland `ext-idle-notify-v1` — works on COSMIC, Sway, and modern Wayland compositors
/// 2. `xprintidle` — works on X11 (returns ms)
/// 3. GNOME Mutter `IdleMonitor.GetIdletime` via DBus — works on GNOME Wayland (returns ms)
/// 4. `org.freedesktop.ScreenSaver.GetSessionIdleTime` via DBus — works on KDE/Sway (returns seconds)
#[cfg(target_os = "linux")]
fn linux_idle_seconds() -> Option<u64> {
    // 1. Wayland ext-idle-notify-v1 (COSMIC, Sway, etc.)
    if let Some(secs) = wayland_idle::get_idle_seconds() {
        return Some(secs);
    }
    // 2. xprintidle (X11)
    if let Some(secs) = linux_xprintidle() {
        return Some(secs);
    }
    // 3. GNOME Mutter IdleMonitor (Wayland)
    if let Some(secs) = linux_mutter_idle() {
        return Some(secs);
    }
    // 4. freedesktop ScreenSaver (KDE/Sway Wayland)
    linux_screensaver_idle()
}

#[cfg(target_os = "linux")]
fn linux_xprintidle() -> Option<u64> {
    let output = std::process::Command::new("xprintidle").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let ms: u64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;
    Some(ms / 1000)
}

#[cfg(target_os = "linux")]
fn linux_mutter_idle() -> Option<u64> {
    let output = std::process::Command::new("dbus-send")
        .args([
            "--print-reply",
            "--dest=org.gnome.Mutter.IdleMonitor",
            "/org/gnome/Mutter/IdleMonitor/Core",
            "org.gnome.Mutter.IdleMonitor.GetIdletime",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    // Output looks like: `method return ...\n   uint64 12345\n`
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("uint64 ") {
            if let Ok(ms) = rest.trim().parse::<u64>() {
                return Some(ms / 1000);
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn linux_screensaver_idle() -> Option<u64> {
    let output = std::process::Command::new("dbus-send")
        .args([
            "--print-reply",
            "--dest=org.freedesktop.ScreenSaver",
            "/org/freedesktop/ScreenSaver",
            "org.freedesktop.ScreenSaver.GetSessionIdleTime",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    // Output: `method return ...\n   uint32 12345\n` (seconds)
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("uint32 ") {
            if let Ok(secs) = rest.trim().parse::<u64>() {
                return Some(secs);
            }
        }
    }
    None
}

/// Emit a presence status change event to the frontend.
fn emit_status_change(app_handle: &tauri::AppHandle, state: &AppState, status: UserStatus) {
    let pk = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let status_str = match status {
        UserStatus::Online => "online",
        UserStatus::Away => "away",
        UserStatus::Busy => "busy",
        UserStatus::Offline => "offline",
    };
    let _ = app_handle.emit(
        "presence-event",
        &crate::channels::PresenceEvent::StatusChanged {
            public_key: pk,
            status: status_str.to_string(),
            status_message: None,
        },
    );
}

/// Start the idle/auto-away background service.
///
/// Polls OS idle time every 30 seconds. When idle time exceeds the configured
/// `auto_away_minutes`, sets status to Away and stores the previous status.
/// When activity resumes, restores the previous status.
pub fn start_idle_service(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
) -> mpsc::Sender<()> {
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

    // On Wayland, start the ext-idle-notify-v1 monitor thread before polling
    #[cfg(target_os = "linux")]
    wayland_idle::try_init();

    tracing::info!("idle service started");

    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = shutdown_rx.recv() => break,
            }

            let auto_away_minutes = read_auto_away_minutes(&app_handle);
            if auto_away_minutes == 0 {
                continue;
            }
            let threshold = u64::from(auto_away_minutes) * 60;

            let Some(idle_secs) = tokio::task::spawn_blocking(get_idle_seconds)
                .await
                .ok()
                .flatten()
            else {
                tracing::warn!("idle service: get_idle_seconds returned None");
                continue;
            };

            let current_status = state.identity.read().as_ref().map(|id| id.status);
            let is_auto_away = state.pre_away_status.read().is_some();

            tracing::debug!(
                idle_secs,
                threshold,
                ?current_status,
                is_auto_away,
                "idle service tick"
            );

            if idle_secs >= threshold
                && current_status == Some(UserStatus::Online)
                && !is_auto_away
            {
                // Activate auto-away
                *state.pre_away_status.write() = Some(UserStatus::Online);
                if let Some(ref mut id) = *state.identity.write() {
                    id.status = UserStatus::Away;
                }
                if let Err(e) =
                    crate::services::presence_service::publish_status(&state, UserStatus::Away)
                        .await
                {
                    tracing::warn!(error = %e, "auto-away: failed to publish Away status");
                }
                emit_status_change(&app_handle, &state, UserStatus::Away);
                tracing::info!(idle_secs, "auto-away activated");
            } else if idle_secs < threshold && is_auto_away {
                // Restore previous status
                let restore = state
                    .pre_away_status
                    .write()
                    .take()
                    .unwrap_or(UserStatus::Online);
                if let Some(ref mut id) = *state.identity.write() {
                    id.status = restore;
                }
                if let Err(e) =
                    crate::services::presence_service::publish_status(&state, restore).await
                {
                    tracing::warn!(error = %e, "auto-away restore: failed to publish status");
                }
                emit_status_change(&app_handle, &state, restore);
                tracing::info!(?restore, "auto-away deactivated");
            }
        }
        tracing::debug!("idle service shut down");
    });

    shutdown_tx
}

fn read_auto_away_minutes(app_handle: &tauri::AppHandle) -> u32 {
    let Ok(store) = app_handle.store("preferences.json") else {
        return 10;
    };
    store
        .get("preferences")
        .and_then(|v| v.get("autoAwayMinutes")?.as_u64())
        .map_or(10, |v| u32::try_from(v).unwrap_or(10))
}
