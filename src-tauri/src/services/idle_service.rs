use std::sync::Arc;

use tauri::Emitter;
use tauri_plugin_store::StoreExt;
use tokio::sync::mpsc;

use crate::state::{AppState, UserStatus};

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

/// macOS: Read `HIDIdleTime` from `IOKit`'s `IOHIDSystem`.
///
/// This is the reliable approach — unlike `CGEventSourceSecondsSinceLastEventType`
/// which resets on system/app events (notifications, timers, presence updates),
/// `HIDIdleTime` only tracks actual hardware input (mouse, keyboard, trackpad).
///
/// Reference: <https://www.dssw.co.uk/blog/2015-01-21-inactivity-and-idle-time/>
/// Implementation based on: <https://github.com/olback/user-idle-rs/blob/master/src/macos_impl.rs>
#[cfg(target_os = "macos")]
fn macos_idle_seconds() -> Option<u64> {
    // ioreg -c IOHIDSystem | grep HIDIdleTime
    // Parse the nanoseconds value from the IOKit registry.
    let output = std::process::Command::new("ioreg")
        .args(["-c", "IOHIDSystem", "-d", "4"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(pos) = line.find("HIDIdleTime") {
            // Line looks like: `"HIDIdleTime" = 1234567890`
            let rest = &line[pos..];
            if let Some(eq_pos) = rest.find('=') {
                let num_str = rest[eq_pos + 1..].trim();
                if let Ok(nanos) = num_str.parse::<u64>() {
                    return Some(nanos / 1_000_000_000);
                }
            }
        }
    }
    None
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
/// 1. `xprintidle` — works on X11 (returns ms)
/// 2. GNOME Mutter `IdleMonitor.GetIdletime` via DBus — works on GNOME Wayland (returns ms)
/// 3. `org.freedesktop.ScreenSaver.GetSessionIdleTime` via DBus — works on KDE/Sway (returns seconds)
#[cfg(target_os = "linux")]
fn linux_idle_seconds() -> Option<u64> {
    // 1. xprintidle (X11)
    if let Some(secs) = linux_xprintidle() {
        return Some(secs);
    }
    // 2. GNOME Mutter IdleMonitor (Wayland)
    if let Some(secs) = linux_mutter_idle() {
        return Some(secs);
    }
    // 3. freedesktop ScreenSaver (KDE/Sway Wayland)
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
                let pk = state
                    .identity
                    .read()
                    .as_ref()
                    .map(|id| id.public_key.clone())
                    .unwrap_or_default();
                let _ = app_handle.emit(
                    "presence-event",
                    &crate::channels::PresenceEvent::StatusChanged {
                        public_key: pk,
                        status: "away".to_string(),
                        status_message: None,
                    },
                );
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
                let pk = state
                    .identity
                    .read()
                    .as_ref()
                    .map(|id| id.public_key.clone())
                    .unwrap_or_default();
                let status_str = match restore {
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
