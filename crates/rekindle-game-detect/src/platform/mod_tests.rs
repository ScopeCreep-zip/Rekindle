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

/// A scan's table holds no `/proc/<pid>/stat` or task stat file once
/// its refresh returns: held files are what pushed the app past 1,024
/// descriptors into the vendored libdbus's `select()` overflow.
#[cfg(target_os = "linux")]
#[test]
fn a_scan_holds_no_proc_stat_files() {
    fn held_stat_files() -> usize {
        std::fs::read_dir("/proc/self/fd")
            .map(|dir| {
                dir.filter_map(Result::ok)
                    .filter_map(|e| std::fs::read_link(e.path()).ok())
                    .filter(|target| {
                        let t = target.to_string_lossy();
                        t.starts_with("/proc/") && t.ends_with("/stat")
                    })
                    .count()
            })
            .unwrap_or(0)
    }
    let _scan = crate::test_support::process_scan();
    let mut sys = super::process_table();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    assert!(!sys.processes().is_empty(), "the scan found processes");
    assert_eq!(
        held_stat_files(),
        0,
        "the table keeps /proc stat files open"
    );
}
