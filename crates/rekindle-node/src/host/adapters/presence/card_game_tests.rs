//! The card's game is what the sharing policy lets out, and nothing
//! while the status hides presence or no game is played.

use rekindle_types::dto::presence::{OwnGame, OwnPresenceView};
use rekindle_types::presence::{IdleState, PresenceSharingPolicy, SessionStatus, ShareScope};

use super::shared_game;

fn playing(status: SessionStatus) -> OwnPresenceView {
    OwnPresenceView {
        status,
        status_message: String::new(),
        game: Some(OwnGame {
            game_id: 4100,
            game_name: "Quake III Arena".into(),
            elapsed_seconds: 42,
            server_address: Some("10.0.0.7:27960".into()),
            icon_key: None,
        }),
        idle: IdleState::Active,
    }
}

fn sharing_activity() -> PresenceSharingPolicy {
    PresenceSharingPolicy {
        share_activity: ShareScope::Members,
        ..PresenceSharingPolicy::default()
    }
}

#[test]
fn a_shared_game_is_carried_without_its_elapsed_time() {
    let game = shared_game(&playing(SessionStatus::Online), &sharing_activity())
        .expect("the game is shared");
    assert_eq!(game.game_name, "Quake III Arena");
    assert_eq!(game.game_id.as_deref(), Some("4100"));
    assert_eq!(game.server_address.as_deref(), Some("10.0.0.7:27960"));
    assert_eq!(game.elapsed_seconds, None);
}

#[test]
fn a_game_the_policy_hides_never_reaches_the_card() {
    let own = playing(SessionStatus::Online);
    assert_eq!(shared_game(&own, &PresenceSharingPolicy::default()), None);
    let offline = PresenceSharingPolicy {
        share_online: false,
        ..sharing_activity()
    };
    assert_eq!(shared_game(&own, &offline), None);
    let invisible = playing(SessionStatus::Invisible);
    assert_eq!(shared_game(&invisible, &sharing_activity()), None);
}

#[test]
fn a_stopped_game_clears_the_card() {
    let mut own = playing(SessionStatus::Online);
    own.game = None;
    assert_eq!(shared_game(&own, &sharing_activity()), None);
}
