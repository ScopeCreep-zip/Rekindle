// macOS-specific process enumeration enhancements.
// The cross-platform sysinfo-based scanner in mod.rs handles the common case.
// This module provides macOS-specific features.

use sysinfo::System;

/// Get the path to a running process executable (macOS-specific).
///
/// This is useful for disambiguating processes with the same name
/// (e.g., multiple "java" processes) by checking the full path.
pub fn get_process_path(pid: u32) -> Option<String> {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let sysinfo_pid = sysinfo::Pid::from_u32(pid);
    sys.process(sysinfo_pid)
        .and_then(|p| p.exe().map(|e| e.to_string_lossy().to_string()))
}

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

/// List running process names with their full executable paths.
///
/// Returns (`process_name`, `exe_path`) pairs for richer game detection.
pub fn list_processes_with_paths() -> Vec<(String, Option<String>)> {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.processes()
        .values()
        .map(|p| {
            let name = p.name().to_string_lossy().to_string();
            let path = p.exe().map(|e| e.to_string_lossy().to_string());
            (name, path)
        })
        .collect()
}
