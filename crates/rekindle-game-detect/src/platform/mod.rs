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

/// List all running process names on the current platform.
pub fn list_process_names() -> Vec<String> {
    use sysinfo::System;

    let mut sys = System::new();
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
    use sysinfo::System;

    let mut sys = System::new();
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
    use sysinfo::System;

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
mod tests {
    use super::alternate_lookup_names;

    /// An ordinary native binary yields no extra candidates, except on
    /// Windows where the other `.exe` spelling is always worth a try.
    #[test]
    fn plain_process_name_is_cheap() {
        let alts = alternate_lookup_names("some-daemon", None, &[]);
        #[cfg(target_os = "windows")]
        assert_eq!(alts, vec!["some-daemon.exe".to_string()]);
        #[cfg(not(target_os = "windows"))]
        assert!(alts.is_empty(), "got {alts:?}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn app_bundle_yields_bundle_name() {
        assert_eq!(
            alternate_lookup_names(
                "steam_osx",
                Some("/Applications/Steam.app/Contents/MacOS/steam_osx"),
                &[],
            ),
            vec!["Steam".to_string()]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn non_bundle_path_yields_nothing() {
        assert!(alternate_lookup_names("ls", Some("/bin/ls"), &[]).is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proton_game_yields_the_windows_exe() {
        let args = vec![
            "wine64".to_string(),
            "Z:\\games\\eldenring.exe".to_string(),
            "--skip-intro".to_string(),
        ];
        assert_eq!(
            alternate_lookup_names("wine64", None, &args),
            vec!["eldenring.exe".to_string()]
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn exe_and_bare_spellings_are_paired() {
        assert_eq!(
            alternate_lookup_names("cs2.exe", None, &[]),
            vec!["cs2".to_string()]
        );
        assert_eq!(
            alternate_lookup_names("cs2", None, &[]),
            vec!["cs2.exe".to_string()]
        );
    }
}
