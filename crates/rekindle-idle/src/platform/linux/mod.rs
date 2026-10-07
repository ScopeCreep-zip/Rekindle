//! Linux idle-time detection: try multiple methods for X11 and Wayland coverage.
//!
//! 1. Wayland `ext-idle-notify-v1` — works on COSMIC, Sway, and modern Wayland compositors
//! 2. `xprintidle` — works on X11 (returns ms)
//! 3. GNOME Mutter `IdleMonitor.GetIdletime` via DBus — works on GNOME Wayland (returns ms)
//! 4. `org.freedesktop.ScreenSaver.GetSessionIdleTime` via DBus — works on KDE/Sway (returns seconds)

pub mod wayland;

pub(crate) fn linux_idle_seconds() -> Option<u64> {
    // 1. Wayland ext-idle-notify-v1 (COSMIC, Sway, etc.) — lazily starts
    // the monitor thread on first call (idempotent, cheap after that) so
    // callers just poll this one function.
    wayland::try_init();
    if let Some(secs) = wayland::get_idle_seconds() {
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
