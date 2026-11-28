//! Hiding our game is prompt (plan E3.4 S6): our card carries the game our
//! sharing consent lets out, so withdrawing the consent alone, with no
//! presence change, rewrites the card within the writer's settle window
//! (`presence::card_game::CARD_GAME_SETTLE`, 5 s) and a member sees the
//! game stop.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, SettingsRequest};
use rekindle_types::presence::{PresenceSharingPolicy, ShareScope};
use rekindle_types::subscription_events::{
    GameActivity, PresenceEvent, PresenceSnapshot, SubscriptionEvent,
};

use crate::harness::{eventually, Community};
use crate::pair::LIMIT;

/// The writer's settle window plus one card write and its watch delivery.
const HIDDEN_WITHIN: Duration = Duration::from_secs(5 + 5);

fn game_of<'a>(event: &'a SubscriptionEvent, member: &str) -> Option<&'a GameActivity> {
    match event {
        SubscriptionEvent::Presence(PresenceEvent::CommunityMemberChanged {
            pseudonym,
            snapshot: PresenceSnapshot { game, .. },
            ..
        }) if pseudonym == member => game.as_ref(),
        _ => None,
    }
}

fn set_policy(community: &str, share_activity: ShareScope) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Settings(
        SettingsRequest::CommunitySetPresencePolicy {
            community: community.to_owned(),
            policy: PresenceSharingPolicy {
                share_activity,
                ..PresenceSharingPolicy::default()
            },
        },
    ))
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn withdrawing_activity_consent_clears_the_card_game_without_a_presence_change() {
    let c = Community::of(2, 9301).await;
    let player = c.pseudonym(0).await;
    let events = c.nodes[1].events().await;
    c.nodes[0]
        .ok(set_policy(&c.anchor, ShareScope::Members))
        .await;
    c.nodes[0]
        .ok(IpcRequest::GamePresenceSet {
            game_name: "CS2".into(),
            game_id: Some(7),
            elapsed_seconds: 0,
            server_address: None,
        })
        .await;
    let playing = || async {
        events.any(|event| {
            matches!(
                game_of(event, &player),
                Some(GameActivity::Playing { game_name, .. }) if game_name == "CS2"
            )
        })
    };
    assert!(
        eventually(&c.sim, LIMIT, playing).await,
        "member1 sees member0's shared game on its card"
    );
    c.nodes[0].ok(set_policy(&c.anchor, ShareScope::None)).await;
    let stopped = || async {
        events.any(|event| matches!(game_of(event, &player), Some(GameActivity::Idle)))
    };
    assert!(
        eventually(&c.sim, HIDDEN_WITHIN, stopped).await,
        "member1 sees the game leave member0's card once member0 stops sharing it"
    );
}
