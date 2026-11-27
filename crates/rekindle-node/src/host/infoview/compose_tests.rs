use rekindle_types::friend::NotJoinableReason;

use super::*;

/// A community's server row always carries `Join`, sourced from the
/// community, even when its game id is not numeric: then it is not
/// joinable and says why; a launchable game on the server is joinable.
#[test]
fn a_community_server_row_joins_from_the_community() {
    let info = |game_id: &str| GameServerInfo {
        id: format!("gs_{}", "0".repeat(32)),
        game_id: game_id.into(),
        label: "Frag Night".into(),
        address: "203.0.113.7:27015".into(),
        added_by: "ab".repeat(32),
        created_at: 1,
    };
    assert_eq!(
        server_row(info("4100")).join,
        InfoviewAction::JoinServer {
            game_id: Some(4_100),
            server: Some("203.0.113.7:27015".into()),
            source: JoinSource::Community,
            joinable: true,
            joinable_reason: None,
        }
    );
    assert_eq!(
        server_row(info("quake")).join,
        InfoviewAction::JoinServer {
            game_id: None,
            server: Some("203.0.113.7:27015".into()),
            source: JoinSource::Community,
            joinable: false,
            joinable_reason: Some(NotJoinableReason::CannotLaunch),
        }
    );
}
