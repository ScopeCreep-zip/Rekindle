// Linux-specific process enumeration enhancements.
// The cross-platform sysinfo-based scanner in mod.rs handles the common case.
// This module provides Linux-specific features.

use std::fs;
use std::path::PathBuf;

/// Resolve the actual executable path for a process via /proc/pid/exe symlink.
///
/// More reliable than process name matching for games run through Wine/Proton.
pub fn resolve_exe_path(pid: u32) -> Option<PathBuf> {
    fs::read_link(format!("/proc/{pid}/exe")).ok()
}

/// Read the full command line of a process from /proc/pid/cmdline.
///
/// Useful for detecting games launched with specific arguments
/// (e.g., Steam games with AppID arguments).
pub fn read_cmdline(pid: u32) -> Option<Vec<String>> {
    let data = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args: Vec<String> = data
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).to_string())
        .collect();
    if args.is_empty() {
        None
    } else {
        Some(args)
    }
}

/// Check if a process is running under Wine/Proton.
///
/// Proton games on Linux run as wine processes with the Windows .exe
/// visible in the command line.
pub fn is_wine_process(pid: u32) -> bool {
    if let Some(cmdline) = read_cmdline(pid) {
        cmdline
            .iter()
            .any(|arg| arg.contains("wine") || arg.contains("proton") || arg.ends_with(".exe"))
    } else {
        false
    }
}

/// Extract the Windows executable name from a Wine/Proton process.
///
/// e.g., ["wine64", "C:\\games\\game.exe", "--args"] -> Some("game.exe")
pub fn extract_wine_exe_name(pid: u32) -> Option<String> {
    let cmdline = read_cmdline(pid)?;
    for arg in &cmdline {
        if arg.ends_with(".exe") {
            // Get just the filename from the Windows-style path
            let name = arg.rsplit(['\\', '/']).next()?;
            return Some(name.to_string());
        }
    }
    None
}

/// List all running PIDs by reading /proc directory entries.
///
/// This is a fast alternative to sysinfo for just getting PID list.
pub fn list_pids() -> Vec<u32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return vec![];
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .collect()
}
