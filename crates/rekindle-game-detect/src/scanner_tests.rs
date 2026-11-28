use super::*;

/// A Source game's `+connect` becomes a joinable `ServerAddress`
/// (plan E7.2 argv fix).
#[test]
fn a_connect_argument_yields_a_server_address() {
    let args: Vec<String> = [
        "hl2.exe",
        "-game",
        "cstrike",
        "+connect",
        "203.0.113.7:27015",
    ]
    .map(String::from)
    .to_vec();
    let rp = presence_from_args(4181, &args).expect("rich presence");
    let server = rp.server_address().expect("a valid address");
    assert_eq!(server.to_string(), "203.0.113.7:27015");
    assert!(presence_from_args(4181, &["hl2.exe".to_string()]).is_none());
}

/// The bug: the names-only table holds no argv; a candidate's command
/// line is loaded by its own refresh (this test process stands in).
#[test]
fn a_candidates_command_line_is_read_after_a_names_only_refresh() {
    let _scan = crate::test_support::process_scan();
    let mut d = detector(exe_only_db());
    let me = sysinfo::get_current_pid().expect("own pid");
    d.system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::new(),
    );
    assert!(d.system.process(me).expect("listed").cmd().is_empty());
    let args = d.command_line(me);
    assert!(!args.is_empty(), "the candidate's argv is read");
}

/// Mirrors the shape of the bundled database's `.exe`-only entries —
/// 11 of the 50 real ones look like this (Elden Ring, Skyrim, Path
/// of Exile, …), which is exactly the set that name matching alone
/// cannot find under Proton.
fn exe_only_db() -> GameDatabase {
    GameDatabase::from_json(
        r#"{"games":[{"id":1,"name":"Elden Ring",
                 "process_names":["eldenring.exe"],"icon":null}]}"#,
    )
    .expect("valid fixture")
}

fn detector(db: GameDatabase) -> GameDetector {
    GameDetector::new(
        db,
        Duration::from_secs(30),
        Arc::new(rekindle_utils::SystemClock),
    )
}

#[test]
fn exact_process_name_still_matches() {
    let d = detector(exe_only_db());
    let hit = d.resolve_entry("eldenring.exe", None, &[]).expect("match");
    assert_eq!(hit.id, 1);
}

#[test]
fn unrelated_process_does_not_match() {
    let d = detector(exe_only_db());
    assert!(d.resolve_entry("bash", Some("/bin/bash"), &[]).is_none());
}

/// The regression this wiring exists for: under Proton the process
/// is `wine64` and the game is named only in the command line.
/// Before `alternate_lookup_names` was wired in, this returned None.
#[cfg(target_os = "linux")]
#[test]
fn proton_wrapped_game_is_found() {
    let d = detector(exe_only_db());
    let args = vec!["wine64".to_string(), "Z:\\games\\eldenring.exe".to_string()];
    let hit = d.resolve_entry("wine64", None, &args).expect("match");
    assert_eq!(hit.name, "Elden Ring");
}

/// macOS equivalent: the process is the inner binary, the database
/// entry is the bundle name.
#[cfg(target_os = "macos")]
#[test]
fn app_bundle_game_is_found() {
    let db = GameDatabase::from_json(
        r#"{"games":[{"id":2,"name":"Steam","process_names":["Steam"],"icon":null}]}"#,
    )
    .expect("valid fixture");
    let d = detector(db);
    assert!(d.resolve_entry("steam_osx", None, &[]).is_none());
    let hit = d
        .resolve_entry(
            "steam_osx",
            Some("/Applications/Steam.app/Contents/MacOS/steam_osx"),
            &[],
        )
        .expect("bundle name must resolve");
    assert_eq!(hit.id, 2);
}

/// Windows: the database may hold either spelling.
#[cfg(target_os = "windows")]
#[test]
fn bare_name_finds_exe_entry() {
    let d = detector(exe_only_db());
    let hit = d.resolve_entry("eldenring", None, &[]).expect("match");
    assert_eq!(hit.id, 1);
}
