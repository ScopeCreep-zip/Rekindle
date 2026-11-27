use super::*;

const INI: &str = "[Modified]\r\nLast=1\r\n\r\n[Version]\r\nVersion=46139\r\n\r\n\
        [-1]\r\nLongName=Unknown Game\r\nShortName=unknown\r\n\r\n\
        [7197_1]\r\nLongName=0 A.D.\r\nShortName=0ad\r\nLauncherExe=pyrogenesis.exe\r\n\r\n\
        [7197_2]\r\nLongName=0 A.D.\r\nShortName=0ad\r\nDetectExe[0]=bin\\\\Pyro2.exe\r\n\
        ;LauncherExe=commented.exe\r\n\r\n\
        [5134]\r\nLongName=Team Fortress 2\r\nShortName=tf2\r\nLauncherExe=steam.exe\r\n\
        DetectExe=tf_win64.exe\r\n";

#[test]
fn variants_merge_under_one_id() {
    let games = read(INI.as_bytes()).unwrap();
    assert_eq!(games.keys().copied().collect::<Vec<_>>(), [5134, 7197]);
    let ad = &games[&7197];
    assert_eq!(ad.short_names, ["0ad"]);
    let exes: Vec<&str> = ad.exes.iter().map(String::as_str).collect();
    assert_eq!(exes, ["pyro2.exe", "pyrogenesis.exe"]);
    assert!(read(b"[abc]\r\nLongName=x\r\n").is_err());
    assert!(read(b"[1]\r\n[1]\r\n").is_err());
    assert!(read(b"[1]\r\nnot a pair\r\n").is_err());
}

#[test]
fn a_game_maps_only_by_name_and_process() {
    let games = read(INI.as_bytes()).unwrap();
    let db = r#"{"games": [
            {"id": 9, "name": "0 A.D.", "process_names": ["Pyrogenesis.exe"]},
            {"id": 4100, "name": "Team Fortress 2", "process_names": ["hl2.exe"]},
            {"id": 7, "name": "Zero AD", "process_names": ["pyrogenesis.exe"]}
        ]}"#;
    let mapped = map(db, &games).unwrap();
    assert_eq!(
        mapped,
        [Mapped {
            id: 9,
            name: "0 A.D.".to_owned(),
            xfire_id: 7197,
            network_args: None,
            unusable_args: Vec::new(),
        }],
        "name alone (TF2) or process alone (Zero AD) does not map"
    );
    assert!(json(&mapped).contains("\"xfire_id\": 7197"));
    assert!(!json(&mapped).contains("network_args"));
}

/// Xfire's `host:port` pair becomes `{addr}`; a value still holding a
/// placeholder Rekindle cannot fill is kept out of the template.
#[test]
fn network_args_take_the_address_once() {
    let ini = "[6957]\r\nLongName=Dota 2\r\nLauncherExe=steam.exe\r\n\
            LauncherNetworkArgs=+connect %UA_GAME_HOST_NAME%:%UA_GAME_HOST_PORT%\r\n\
            [2]\r\nLongName=P\r\nLauncherExe=p.exe\r\n\
            LauncherNetworkArgs=%UA_GAME_HOST_NAME%:%UA_GAME_HOST_PORT%%UA_LAUNCHER_PASSWORD_ARGS%\r\n";
    let games = read(ini.as_bytes()).unwrap();
    let dota: Vec<&str> = games[&6957]
        .network_args
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(dota, ["+connect {addr}"]);
    assert!(games[&2].network_args.is_empty());
    assert_eq!(games[&2].unusable_args.len(), 1);
    let db = r#"{"games": [{"id": 4102, "name": "Dota 2", "process_names": ["steam.exe"]}]}"#;
    let mapped = map(db, &games).unwrap();
    assert!(json(&mapped).contains("\"network_args\": \"+connect {addr}\""));
}
