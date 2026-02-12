// Windows-specific process enumeration enhancements.
// The cross-platform sysinfo-based scanner in mod.rs handles the common case.
// This module provides Windows-specific features.

use sysinfo::System;

/// Get the path to a running process executable (Windows-specific).
///
/// Uses sysinfo which internally uses CreateToolhelp32Snapshot on Windows.
pub fn get_process_path(pid: u32) -> Option<String> {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let sysinfo_pid = sysinfo::Pid::from_u32(pid);
    sys.process(sysinfo_pid)
        .and_then(|p| p.exe().map(|e| e.to_string_lossy().to_string()))
}

/// List running process names with their full executable paths (Windows).
///
/// Returns (process_name, exe_path) pairs for richer game detection.
/// On Windows, sysinfo uses CreateToolhelp32Snapshot internally.
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
