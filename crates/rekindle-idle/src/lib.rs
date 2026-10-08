//! Cross-platform OS idle-time detection.
//!
//! Harvested out of `src-tauri/src/services/idle_service.rs` (audit:
//! `docs/research/2026-10-harvest-security-infra-audit.md`) — this crate
//! holds exactly the platform FFI that file used to carry inline, with
//! zero `AppState`/Tauri dependency, mirroring `rekindle-game-detect`'s
//! `platform/` module shape. The orchestration that polls this on a
//! timer and flips presence status to Away/back stays in src-tauri; see
//! `services::idle_service::start_idle_service`.

mod platform;

/// Get system idle time in seconds (platform-specific).
///
/// `None` when no detection method succeeded on this platform/session
/// (e.g. no X11/Wayland idle protocol available, or the macOS/Windows
/// FFI call itself failed). On Linux this lazily starts the
/// `ext-idle-notify-v1` Wayland monitor thread on first call, if running
/// under Wayland — idempotent, safe to call on every poll tick.
#[must_use]
pub fn get_idle_seconds() -> Option<u64> {
    platform::get_idle_seconds()
}
