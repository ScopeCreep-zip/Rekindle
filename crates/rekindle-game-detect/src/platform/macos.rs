//! macOS-specific process enumeration.
//!
//! The cross-platform `sysinfo` accessors live in `mod.rs`; only the
//! genuinely macOS-shaped helpers belong here.

/// Check if a process is an application bundle (.app) on macOS.
///
/// Games installed via Steam or native macOS often run as .app bundles.
/// The executable lives inside `Name.app/Contents/MacOS/`.
pub fn is_app_bundle(process_path: &str) -> bool {
    process_path.contains(".app/Contents/MacOS/")
}

/// Extract the bundle name from a macOS .app path.
///
/// e.g., `"/Applications/Steam.app/Contents/MacOS/steam_osx"` -> `"Steam"`
pub fn extract_bundle_name(process_path: &str) -> Option<String> {
    let app_idx = process_path.find(".app/")?;
    let before_app = &process_path[..app_idx];
    let last_slash = before_app.rfind('/').map_or(0, |i| i + 1);
    Some(before_app[last_slash..].to_string())
}
