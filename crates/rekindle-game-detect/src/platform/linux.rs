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
    read_cmdline(pid).is_some_and(|cmdline| is_wine_cmdline(&cmdline))
}

/// [`is_wine_process`] against a command line the caller already has.
///
/// The scanner gets each process's argv from `sysinfo` as it walks the
/// process table, so it uses this instead of re-reading `/proc/pid/cmdline`
/// — one less syscall per process per scan, and no race against a
/// process that exits mid-scan.
pub fn is_wine_cmdline(args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg.contains("wine") || arg.contains("proton") || has_exe_extension(arg))
}

/// Windows paths are case-insensitive — `GAME.EXE` and `game.exe`
/// name the same binary, so match the extension case-insensitively.
fn has_exe_extension(arg: &str) -> bool {
    arg.len() >= 4 && arg[arg.len() - 4..].eq_ignore_ascii_case(".exe")
}

/// Extract the Windows executable name from a Wine/Proton process.
///
/// e.g., ["wine64", "C:\\games\\game.exe", "--args"] -> Some("game.exe")
pub fn extract_wine_exe_name(pid: u32) -> Option<String> {
    wine_exe_from_cmdline(&read_cmdline(pid)?)
}

/// [`extract_wine_exe_name`] against a command line the caller already
/// has — see [`is_wine_cmdline`] for why the scanner prefers this.
///
/// Returns the bare filename, so both `Z:\games\eldenring.exe` and
/// `/home/u/.steam/.../eldenring.exe` yield `eldenring.exe`, which is
/// the spelling the game database stores.
pub fn wine_exe_from_cmdline(args: &[String]) -> Option<String> {
    args.iter()
        .find(|arg| has_exe_extension(arg))
        .and_then(|arg| arg.rsplit(['\\', '/']).next())
        .map(std::string::ToString::to_string)
}

/// List all running PIDs by reading /proc directory entries.
///
/// This is a fast alternative to sysinfo for just getting PID list.
pub fn list_pids() -> Vec<u32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return vec![];
    };
    entries
        .filter_map(std::result::Result::ok)
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .collect()
}
