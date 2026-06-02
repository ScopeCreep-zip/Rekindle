//! Platform-specific environment setup that must run before `tauri::Builder`.

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
pub fn linux_display_setup() {
    use std::path::Path;

    // Wayland display discovery
    if std::env::var("WAYLAND_DISPLAY")
        .unwrap_or_default()
        .is_empty()
    {
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
