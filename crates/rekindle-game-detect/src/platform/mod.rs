//! Platform-specific process-enumeration enhancements.
//!
//! Matching a running process against the game database by name alone
//! is not enough on any of the three platforms:
//!
//! * **Linux** — Windows games run under Wine/Proton, so the process is
//!   `wine64` or `proton` and the real executable appears only in the
//!   command line. 11 of the 50 bundled games list *only* `.exe`
//!   process names (Elden Ring, Skyrim, Path of Exile, …), so under
//!   Proton they could not be detected by name at all.
//! * **macOS** — games ship as `.app` bundles and the process name is
//!   the inner binary: `Steam.app/Contents/MacOS/steam_osx` runs as
//!   `steam_osx`, not `Steam`.
//! * **Windows** — the database may list `csgo.exe` or `csgo`.
//!
//! [`alternate_lookup_names`] turns those quirks into extra candidate
//! names, and `scanner.rs` tries them when the raw name misses.

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod linux;

/// A process table for one scan. `sysinfo` keeps a `/proc` file open per
/// indexed process and thread on Linux, up to half `RLIMIT_NOFILE`, to
/// speed up later refreshes (`sysinfo` 0.32.1 `lib.rs:113-118`). A scan
/// here builds a fresh table each time, so that cache buys nothing, and it
/// pushed the app past 1,024 descriptors within seconds of start (1,195
/// thread and 362 process stat files on the Linux machine, 2026-11-17),
/// where the vendored libdbus's `select()`-based `_dbus_poll` overflows its
/// stack `fd_set` and the process aborts ("stack smashing detected"). The
/// limit is set to 0 once, before the first refresh, as `sysinfo` requires.
pub(crate) fn process_table() -> sysinfo::System {
    static NO_HELD_FILES: std::sync::Once = std::sync::Once::new();
    NO_HELD_FILES.call_once(|| {
        // `false` off Linux, where nothing is held; nothing to report.
        let _ = sysinfo::set_open_files_limit(0);
    });
    sysinfo::System::new()
}

/// List all running process names on the current platform.
pub fn list_process_names() -> Vec<String> {
    let mut sys = process_table();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    sys.processes()
        .values()
        .map(|p| p.name().to_string_lossy().to_string())
        .collect()
}

/// Get the executable path for a process.
///
/// Useful for disambiguating processes that share a name (several
/// `java` processes, say) by their full path.
///
/// This is `sysinfo`, which is cross-platform — it lived in both
/// `macos.rs` and `windows.rs` as byte-identical copies, each with a
/// doc comment claiming platform-specific behaviour it did not have
/// (the Windows copy credited `CreateToolhelp32Snapshot`, which
/// `sysinfo` uses internally but this code does not name).
pub fn get_process_path(pid: u32) -> Option<String> {
    let mut sys = process_table();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let sysinfo_pid = sysinfo::Pid::from_u32(pid);
    sys.process(sysinfo_pid)
        .and_then(|p| p.exe().map(|e| e.to_string_lossy().to_string()))
}

/// List running process names with their full executable paths.
///
/// Returns `(process_name, exe_path)` pairs for richer game detection
/// than [`list_process_names`] alone. Same story as
/// [`get_process_path`] — one cross-platform implementation, not two.
pub fn list_processes_with_paths() -> Vec<(String, Option<String>)> {
    let mut sys = process_table();
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

/// Whether a process of this name names its game only in its arguments,
/// so the scanner reads its command line before matching: a Wine/Proton
/// loader on Linux (see [`alternate_lookup_names`]); nothing elsewhere.
/// Every other process's argv is never read.
pub(crate) fn game_named_in_args(process_name: &str) -> bool {
    cfg!(target_os = "linux") && {
        let name = process_name.to_ascii_lowercase();
        name.contains("wine") || name.contains("proton")
    }
}

/// Extra names to try against the game database when the raw process
/// name does not match, derived from platform conventions.
///
/// The caller tries the raw name first; these are fallbacks, so an
/// empty vec is the normal result for an ordinary native process.
#[allow(
    unused_variables,
    reason = "each platform uses a different subset of the inputs; the signature is shared"
)]
#[must_use]
pub fn alternate_lookup_names(
    process_name: &str,
    exe_path: Option<&str>,
    cmd_args: &[String],
) -> Vec<String> {
    let mut out = Vec::new();

    #[cfg(target_os = "linux")]
    {
        // A Proton/Wine game presents as `wine64`; the Windows binary is
        // named only in the command line. Without this the .exe-only
        // database entries are undetectable on Linux.
        if linux::is_wine_cmdline(cmd_args) {
            if let Some(exe) = linux::wine_exe_from_cmdline(cmd_args) {
                out.push(exe);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        // `Steam.app/Contents/MacOS/steam_osx` -> also try `Steam`.
        if let Some(bundle) = exe_path
            .filter(|p| macos::is_app_bundle(p))
            .and_then(macos::extract_bundle_name)
        {
            out.push(bundle);
        }
    }

    #[cfg(target_os = "windows")]
    {
        // The database may hold either spelling; try the other one.
        if windows::is_executable(process_name) {
            out.push(windows::strip_exe_extension(process_name).to_string());
        } else {
            out.push(format!("{process_name}.exe"));
        }
    }

    out
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
