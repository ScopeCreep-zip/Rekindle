use super::*;

/// The Chess entry's id: the database's free range (Xfire's own list
/// has no native Chess; its 5034 is Windows' Chess Titans).
const CHESS: u32 = 6200;
const CS2: u32 = 4181;
const DOTA2: u32 = 4102;
const TERRARIA: u32 = 5002;

fn db() -> GameDatabase {
    GameDatabase::from_json(
        r#"{"games":[
                {"id":1,"name":"Steam Game","process_names":["a.exe"],"steam_app_id":730},
                {"id":2,"name":"Templated","process_names":["b.exe"],
                 "connect_template":"q3://{addr}"},
                {"id":3,"name":"Neither","process_names":["c.exe"]}
            ]}"#,
    )
    .unwrap()
}

/// [`launch_target`] on macOS, as the URI it opens.
fn uri(
    db: &GameDatabase,
    game_id: u32,
    server: Option<&ServerAddress>,
) -> Result<String, GameDetectError> {
    match launch_target(db, game_id, server, SteamLayout::MacOs)? {
        LaunchTarget::Uri(uri) => Ok(uri),
        other => panic!("not a URI: {other:?}"),
    }
}

fn apps(system: &Path) -> AppRoots {
    AppRoots {
        home: system.join("home"),
        program_files_x86: None,
        system: system.to_path_buf(),
    }
}

fn server() -> ServerAddress {
    ServerAddress::parse("1.2.3.4:27015").unwrap()
}

#[test]
fn launch_unknown_game_errors() {
    assert!(uri(&GameDatabase::empty(), 9999, Some(&server())).is_err());
}

/// A Steam id alone starts the game, never joins a server:
/// `steam://connect` reaches only Source and GoldSrc games (plan E7.2
/// G2), so no blanket connect URI is made up.
#[test]
fn a_steam_id_alone_runs_but_joins_no_server() {
    assert!(uri(&db(), 1, Some(&server())).is_err());
    assert_eq!(uri(&db(), 1, None).unwrap(), "steam://rungameid/730");
}

/// Dota 2 joins with Xfire's `+connect {addr}` over `steam://run`;
/// Terraria (a Steam id, no network arguments) cannot; Counter-Strike 2
/// keeps its own connect template.
#[test]
fn a_server_launch_uses_the_template_or_xfire_network_args() {
    let db = GameDatabase::bundled();
    assert_eq!(
        uri(&db, DOTA2, Some(&server())).unwrap(),
        "steam://run/570//+connect%201.2.3.4:27015/"
    );
    assert!(uri(&db, TERRARIA, Some(&server())).is_err());
    assert_eq!(
        not_joinable(&db, TERRARIA, Some("1.2.3.4:7777")),
        Some(NotJoinableReason::CannotLaunch)
    );
    assert_eq!(
        uri(&db, CS2, Some(&server())).unwrap(),
        "steam://connect/1.2.3.4:27015"
    );
    assert!(joins_servers(db.lookup_by_id(DOTA2).unwrap()));
    assert!(!joins_servers(db.lookup_by_id(TERRARIA).unwrap()));
}

#[test]
fn a_template_takes_only_the_parsed_address() {
    assert_eq!(
        uri(&db(), 2, Some(&server())).unwrap(),
        "q3://1.2.3.4:27015"
    );
    // A template alone cannot start the game without a server.
    assert!(uri(&db(), 2, None).is_err());
}

#[test]
fn a_game_with_no_method_is_refused() {
    assert!(uri(&db(), 3, Some(&server())).is_err());
    assert!(uri(&db(), 3, None).is_err());
    let none = apps(Path::new("/nonexistent-rekindle-root"));
    assert!(!launchable(
        db().lookup_by_id(3).unwrap(),
        SteamLayout::CURRENT,
        &none
    ));
    assert!(launchable(
        db().lookup_by_id(1).unwrap(),
        SteamLayout::CURRENT,
        &none
    ));
}

/// "Join Game" is offered exactly when the launch would run (walk
/// B1): TF2 (4100) on a server joins over Steam; Minecraft (4300) has
/// no way to start on a server; no server is its own reason.
#[test]
fn joinable_follows_the_launch_check() {
    let db = GameDatabase::bundled();
    assert_eq!(not_joinable(&db, 4100, Some("1.2.3.4:27015")), None);
    assert_eq!(
        not_joinable(&db, 4300, Some("1.2.3.4:25565")),
        Some(NotJoinableReason::CannotLaunch)
    );
    assert_eq!(
        not_joinable(&db, 4100, None),
        Some(NotJoinableReason::NoServer)
    );
    assert_eq!(
        not_joinable(&db, 9999, Some("1.2.3.4:27015")),
        Some(NotJoinableReason::CannotLaunch)
    );
}

/// The native Chess (plan E7.15 CH1) starts from the database's fixed
/// path on each OS: launch services open the bundle on macOS, its
/// executable runs on Linux; Windows has no native copy; a server is
/// refused. Nothing is spawned here.
#[test]
fn chess_launches_its_native_copy_per_os() {
    let db = GameDatabase::bundled();
    assert_eq!(
        launch_target(&db, CHESS, None, SteamLayout::MacOs).unwrap(),
        LaunchTarget::Bundle(PathBuf::from("/System/Applications/Chess.app"))
    );
    assert_eq!(
        launch_target(&db, CHESS, None, SteamLayout::Linux).unwrap(),
        LaunchTarget::Exec(PathBuf::from("/usr/games/gnome-chess"))
    );
    assert!(launch_target(&db, CHESS, None, SteamLayout::Windows).is_err());
    assert!(launch_target(&db, CHESS, Some(&server()), SteamLayout::MacOs).is_err());
    assert_eq!(
        under_system(Path::new("/root"), Path::new("/usr/games/gnome-chess")),
        PathBuf::from("/root/usr/games/gnome-chess")
    );
}

/// Chess is launchable only where its native copy is present under
/// the Host's system root: the macOS bundle must name `com.apple.Chess`,
/// the Linux executable must exist.
#[test]
fn chess_is_launchable_where_its_path_exists() {
    let db = GameDatabase::bundled();
    let chess = db.lookup_by_id(CHESS).unwrap();
    let root = tempfile::tempdir().unwrap();
    let apps = apps(root.path());
    assert!(!launchable(chess, SteamLayout::MacOs, &apps));
    assert!(!launchable(chess, SteamLayout::Linux, &apps));

    let contents = root.path().join("System/Applications/Chess.app/Contents");
    std::fs::create_dir_all(&contents).unwrap();
    std::fs::write(
        contents.join("Info.plist"),
        "<string>com.apple.Other</string>",
    )
    .unwrap();
    assert!(!launchable(chess, SteamLayout::MacOs, &apps));
    std::fs::write(
        contents.join("Info.plist"),
        "<string>com.apple.Chess</string>",
    )
    .unwrap();
    assert!(launchable(chess, SteamLayout::MacOs, &apps));

    let games = root.path().join("usr/games");
    std::fs::create_dir_all(&games).unwrap();
    std::fs::write(games.join("gnome-chess"), b"").unwrap();
    assert!(launchable(chess, SteamLayout::Linux, &apps));
    assert!(!launchable(chess, SteamLayout::Windows, &apps));
}
