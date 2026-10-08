//! Per-OS idle-time dispatch. One real implementation per target;
//! mirrors `rekindle-game-detect/src/platform/`'s shape.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// Get system idle time in seconds (platform-specific).
pub(crate) fn get_idle_seconds() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        macos::macos_idle_seconds()
    }
    #[cfg(target_os = "windows")]
    {
        windows::windows_idle_seconds()
    }
    #[cfg(target_os = "linux")]
    {
        linux::linux_idle_seconds()
    }
}
