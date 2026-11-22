//! Game launch support via URI schemes, the one launch path (lead decision
//! (50)): the Host's `GameLaunch` calls [`launch`] for every frontend.
//!
//! Uses `steam://connect/{addr}` for Steam games and custom `connect_template`
//! URIs from the game database for non-Steam games, and
//! `steam://rungameid/{appid}` to start a game without a server. The `open`
//! crate invokes the OS default handler, so Steam handles `steam://` URIs
//! cross-platform.
//!
//! The address comes from a peer (presence, a community's server list), so
//! only a parsed [`ServerAddress`] is ever substituted into the URI (plan
//! E1.1): a raw string could carry a path, a query or a `+exec` argument
//! into what the OS opens.

use std::path::{Path, PathBuf};

use rekindle_utils::paths::AppRoots;

use crate::database::{GameDatabase, GameEntry};
use crate::error::GameDetectError;
use crate::installed::SteamLayout;

pub use rekindle_types::friend::NotJoinableReason;
pub use rekindle_types::game_server::{ServerAddress, ServerAddressError};

/// What the OS is asked to start for a launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchTarget {
    /// A URI the OS's default handler opens (`steam://`, a template's).
    Uri(String),
    /// A macOS app bundle at this fixed database path, opened by launch
    /// services (plan E7.15 CH1).
    Bundle(PathBuf),
    /// An executable at this fixed database path, run with no arguments
    /// and no shell (plan E7.15 CH1).
    Exec(PathBuf),
}

/// What launches `game_id` on `layout`'s OS, connected to `server` when
/// given.
///
/// With a server: the entry's `connect_template`, else
/// `steam://connect/{addr}` when it has a Steam id. Without one:
/// `steam://rungameid/{appid}`, else the OS's native copy from the
/// database ([`GameEntry::native`]).
///
/// # Errors
/// The game is not in `db`, or it has no launch method for this request.
pub fn launch_target(
    db: &GameDatabase,
    game_id: u32,
    server: Option<&ServerAddress>,
    layout: SteamLayout,
) -> Result<LaunchTarget, GameDetectError> {
    let entry = db
        .lookup_by_id(game_id)
        .ok_or_else(|| GameDetectError::DatabaseError(format!("unknown game_id {game_id}")))?;
    let target = match (server, &entry.connect_template, entry.steam_app_id) {
        (Some(server), Some(template), _) => Some(LaunchTarget::Uri(
            template.replace("{addr}", server.as_str()),
        )),
        (Some(server), None, Some(_)) => {
            Some(LaunchTarget::Uri(format!("steam://connect/{server}")))
        }
        (None, _, Some(app_id)) => Some(LaunchTarget::Uri(format!("steam://rungameid/{app_id}"))),
        (None, _, None) => native_target(entry, layout),
        (Some(_), None, None) => None,
    };
    target.ok_or_else(|| GameDetectError::DatabaseError("no launch method for this game".into()))
}

/// Why a friend's `game_id` on `server` cannot be joined here, `None`
/// when it can (plan E7.15 IV3c, walk B1): the same [`launch_target`]
/// check the Host's validation makes before a `GameLaunch`, so "Join
/// Game" is offered exactly when it would run.
#[must_use]
pub fn not_joinable(
    db: &GameDatabase,
    game_id: u32,
    server: Option<&str>,
) -> Option<NotJoinableReason> {
    let Some(server) = server.and_then(|s| ServerAddress::parse(s).ok()) else {
        return Some(NotJoinableReason::NoServer);
    };
    launch_target(db, game_id, Some(&server), SteamLayout::CURRENT)
        .err()
        .map(|_| NotJoinableReason::CannotLaunch)
}

/// The native copy of `entry` on `layout`'s OS, at its database path.
fn native_target(entry: &GameEntry, layout: SteamLayout) -> Option<LaunchTarget> {
    let native = entry.native.as_ref()?;
    match layout {
        SteamLayout::MacOs => native
            .macos
            .as_ref()
            .map(|app| LaunchTarget::Bundle(PathBuf::from(&app.path))),
        SteamLayout::Linux => native
            .linux
            .as_ref()
            .map(|app| LaunchTarget::Exec(PathBuf::from(&app.path))),
        SteamLayout::Windows => None,
    }
}

/// A database path read under `system` ([`AppRoots::system`], `/` on a
/// real install).
#[must_use]
pub fn under_system(system: &Path, path: &Path) -> PathBuf {
    system.join(path.strip_prefix("/").unwrap_or(path))
}

/// Whether the native copy of `entry` is on this machine: the bundle at
/// its path names its bundle id (macOS), or its executable exists
/// (Linux), read under `apps.system`.
#[must_use]
pub fn native_present(entry: &GameEntry, layout: SteamLayout, apps: &AppRoots) -> bool {
    let Some(native) = entry.native.as_ref() else {
        return false;
    };
    match layout {
        SteamLayout::MacOs => native.macos.as_ref().is_some_and(|app| {
            let plist =
                under_system(&apps.system, Path::new(&app.path)).join("Contents/Info.plist");
            std::fs::read(plist).is_ok_and(|bytes| {
                bytes
                    .windows(app.bundle_id.len())
                    .any(|w| w == app.bundle_id.as_bytes())
            })
        }),
        SteamLayout::Linux => native
            .linux
            .as_ref()
            .is_some_and(|app| under_system(&apps.system, Path::new(&app.path)).is_file()),
        SteamLayout::Windows => false,
    }
}

/// Whether `entry` can be launched without a server on `layout`'s OS: it
/// has a Steam id, or its native copy is present under `apps`.
#[must_use]
pub fn launchable(entry: &GameEntry, layout: SteamLayout, apps: &AppRoots) -> bool {
    entry.steam_app_id.is_some() || native_present(entry, layout, apps)
}

/// Launch `game_id`, connected to `server` when given: the OS opens
/// [`launch_target`]'s URI or bundle, or runs its executable, every path
/// read under `apps.system`.
///
/// # Errors
/// [`launch_target`]'s, or the OS could not open or run it.
pub fn launch(
    db: &GameDatabase,
    apps: &AppRoots,
    game_id: u32,
    server: Option<&ServerAddress>,
) -> Result<(), GameDetectError> {
    match launch_target(db, game_id, server, SteamLayout::CURRENT)? {
        LaunchTarget::Uri(uri) => open::that(&uri).map_err(GameDetectError::Io),
        LaunchTarget::Bundle(path) => {
            open::that(under_system(&apps.system, &path)).map_err(GameDetectError::Io)
        }
        LaunchTarget::Exec(path) => {
            let mut child = std::process::Command::new(under_system(&apps.system, &path))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(GameDetectError::Io)?;
            // Reap it when the game exits, so no zombie outlives it.
            std::thread::spawn(move || child.wait());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Chess entry's id: the database's free range (Xfire's own list
    /// has no native Chess; its 5034 is Windows' Chess Titans).
    const CHESS: u32 = 6200;

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

    #[test]
    fn a_steam_game_connects_or_runs_by_app_id() {
        assert_eq!(
            uri(&db(), 1, Some(&server())).unwrap(),
            "steam://connect/1.2.3.4:27015"
        );
        assert_eq!(uri(&db(), 1, None).unwrap(), "steam://rungameid/730");
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
}
