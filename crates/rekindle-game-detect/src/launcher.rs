//! Game launch support via URI schemes, the one launch path (lead decision
//! (50)): the Host's `GameLaunch` calls [`launch`] for every frontend.
//!
//! To a server: the game's own `connect_template` URI from the database
//! (Counter-Strike's `steam://connect/{addr}`: `steam://connect` reaches
//! only Source and GoldSrc games), else, for a Steam game with Xfire's
//! `LauncherNetworkArgs` ([`GameEntry::network_args`]), `steam://run` with
//! those arguments; any other game cannot be launched to a server (plan
//! E7.2 G2). Without a server: `steam://rungameid/{appid}`, else the OS's
//! native copy. The `open` crate invokes the OS default handler, so Steam
//! handles `steam://` URIs cross-platform.
//!
//! The address comes from a peer (presence, a community's server list), so
//! only a parsed [`ServerAddress`] is ever substituted into the URI (plan
//! E1.1): a raw string could carry a path, a query or a `+exec` argument
//! into what the OS opens.

use std::fmt::Write as _;
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

/// `args` percent-encoded for a `steam://run` path segment: every byte
/// but ASCII letters, digits and `-._~+:` (a space becomes `%20`).
fn steam_args(args: &str) -> String {
    let mut out = String::with_capacity(args.len());
    for b in args.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~+:".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// The URI that puts `entry` on `server`, `None` when it has no way.
///
/// `steam://run/<id>//<args>/` is the Steam browser protocol's "run with
/// arguments" form (Valve Developer Community, "Steam browser protocol",
/// <https://developer.valvesoftware.com/wiki/Steam_browser_protocol>): the
/// empty segment between the two slashes is the language, the next the
/// URL-encoded command line. The form is corroborated by community reports,
/// pending verification on a Steam install (owner, 2026-11-23).
fn server_uri(entry: &GameEntry, server: &ServerAddress) -> Option<String> {
    if let Some(template) = &entry.connect_template {
        return Some(template.replace("{addr}", server.as_str()));
    }
    let (app_id, args) = (entry.steam_app_id?, entry.network_args.as_ref()?);
    let args = steam_args(&args.replace("{addr}", server.as_str()));
    Some(format!("steam://run/{app_id}//{args}/"))
}

/// Whether `entry` can be launched to a server: a connect template, or a
/// Steam id with Xfire's network arguments ([`launch_target`]'s condition).
#[must_use]
pub fn joins_servers(entry: &GameEntry) -> bool {
    entry.connect_template.is_some()
        || (entry.steam_app_id.is_some() && entry.network_args.is_some())
}

/// What launches `game_id` on `layout`'s OS, connected to `server` when
/// given.
///
/// With a server: the entry's `connect_template`, else `steam://run` with
/// its network arguments ([`server_uri`]). Without one:
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
    let target = match (server, entry.steam_app_id) {
        (Some(server), _) => server_uri(entry, server).map(LaunchTarget::Uri),
        (None, Some(app_id)) => Some(LaunchTarget::Uri(format!("steam://rungameid/{app_id}"))),
        (None, None) => native_target(entry, layout),
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
#[path = "launcher_tests.rs"]
mod tests;
