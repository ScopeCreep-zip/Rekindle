//! Windows-specific process enumeration.
//!
//! The cross-platform `sysinfo` accessors live in `mod.rs`; only the
//! genuinely Windows-shaped helpers belong here.

/// Check if a process name ends with .exe (Windows convention).
///
/// Useful for filtering game processes from system services.
pub fn is_executable(name: &str) -> bool {
    name.to_lowercase().ends_with(".exe")
}

/// Strip the .exe extension from a process name for matching.
///
/// e.g., "csgo.exe" -> "csgo"
pub fn strip_exe_extension(name: &str) -> &str {
    name.strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".EXE"))
        .unwrap_or(name)
}
