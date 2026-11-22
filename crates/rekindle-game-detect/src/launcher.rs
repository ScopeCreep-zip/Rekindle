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

use crate::database::{GameDatabase, GameEntry};
use crate::error::GameDetectError;

pub use rekindle_types::game_server::{ServerAddress, ServerAddressError};

/// The URI that launches `game_id`, connected to `server` when given.
///
/// With a server: the entry's `connect_template`, else
/// `steam://connect/{addr}` when it has a Steam id. Without one:
/// `steam://rungameid/{appid}`.
///
/// # Errors
/// The game is not in `db`, or it has no launch method for this request.
pub fn launch_uri(
    db: &GameDatabase,
    game_id: u32,
    server: Option<&ServerAddress>,
) -> Result<String, GameDetectError> {
    let entry = db
        .lookup_by_id(game_id)
        .ok_or_else(|| GameDetectError::DatabaseError(format!("unknown game_id {game_id}")))?;
    let uri = match (server, &entry.connect_template, entry.steam_app_id) {
        (Some(server), Some(template), _) => Some(template.replace("{addr}", server.as_str())),
        (Some(server), None, Some(_)) => Some(format!("steam://connect/{server}")),
        (None, _, Some(app_id)) => Some(format!("steam://rungameid/{app_id}")),
        (Some(_), None, None) | (None, _, None) => None,
    };
    uri.ok_or_else(|| GameDetectError::DatabaseError("no launch method for this game".into()))
}

/// Whether `entry` can be launched without a server.
#[must_use]
pub const fn launchable(entry: &GameEntry) -> bool {
    entry.steam_app_id.is_some()
}

/// Launch `game_id`, connected to `server` when given: the OS opens
/// [`launch_uri`].
///
/// # Errors
/// [`launch_uri`]'s, or the OS could not open the URI.
pub fn launch(
    db: &GameDatabase,
    game_id: u32,
    server: Option<&ServerAddress>,
) -> Result<(), GameDetectError> {
    let uri = launch_uri(db, game_id, server)?;
    open::that(&uri).map_err(GameDetectError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn server() -> ServerAddress {
        ServerAddress::parse("1.2.3.4:27015").unwrap()
    }

    #[test]
    fn launch_unknown_game_errors() {
        assert!(launch_uri(&GameDatabase::empty(), 9999, Some(&server())).is_err());
    }

    #[test]
    fn a_steam_game_connects_or_runs_by_app_id() {
        assert_eq!(
            launch_uri(&db(), 1, Some(&server())).unwrap(),
            "steam://connect/1.2.3.4:27015"
        );
        assert_eq!(launch_uri(&db(), 1, None).unwrap(), "steam://rungameid/730");
    }

    #[test]
    fn a_template_takes_only_the_parsed_address() {
        assert_eq!(
            launch_uri(&db(), 2, Some(&server())).unwrap(),
            "q3://1.2.3.4:27015"
        );
        // A template alone cannot start the game without a server.
        assert!(launch_uri(&db(), 2, None).is_err());
    }

    #[test]
    fn a_game_with_no_method_is_refused() {
        assert!(launch_uri(&db(), 3, Some(&server())).is_err());
        assert!(launch_uri(&db(), 3, None).is_err());
        assert!(!launchable(db().lookup_by_id(3).unwrap()));
        assert!(launchable(db().lookup_by_id(1).unwrap()));
    }
}
