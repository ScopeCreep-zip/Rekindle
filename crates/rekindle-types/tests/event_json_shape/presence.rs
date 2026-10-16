//! `PresenceEvent` samples, covering both `GameActivity` variants.

use rekindle_types::subscription_events::{
    GameActivity, PresenceEvent, PresenceSnapshot, SubscriptionEvent,
};

use crate::{community, pk, text, Sample};

pub const NAMES: &[&str] = &["communityMemberChanged", "friendChanged", "selfChanged"];

pub const NESTED: &[&str] = &["GameActivity.idle", "GameActivity.playing"];

pub fn variant_name(e: &PresenceEvent) -> &'static str {
    match e {
        PresenceEvent::CommunityMemberChanged { .. } => "communityMemberChanged",
        PresenceEvent::FriendChanged { .. } => "friendChanged",
        PresenceEvent::SelfChanged { .. } => "selfChanged",
    }
}

fn game_name(game: &GameActivity) -> &'static str {
    match game {
        GameActivity::Idle => "GameActivity.idle",
        GameActivity::Playing { .. } => "GameActivity.playing",
    }
}

pub fn nested_names(e: &PresenceEvent) -> Vec<&'static str> {
    e.snapshot().game.iter().map(game_name).collect()
}

fn ev(name: &str, e: PresenceEvent) -> Sample {
    (format!("presence.{name}"), SubscriptionEvent::Presence(e))
}

fn playing() -> PresenceSnapshot {
    PresenceSnapshot::status("online")
        .with_status_message(Some(text("fragging")))
        .with_game(GameActivity::Playing {
            game_name: text("Battlefield 2"),
            game_id: Some(4_101),
            elapsed_seconds: Some(1_234),
            server_address: Some(text("10.0.0.1:16567")),
        })
}

pub fn samples() -> Vec<Sample> {
    vec![
        ev(
            "communityMemberChanged",
            PresenceEvent::CommunityMemberChanged {
                community: community(),
                pseudonym: pk('a'),
                snapshot: playing(),
            },
        ),
        ev(
            "friendChanged",
            PresenceEvent::FriendChanged {
                peer_key: pk('b'),
                snapshot: PresenceSnapshot::status("away")
                    .with_status_message(Some(text("brb")))
                    .with_game(GameActivity::Idle),
            },
        ),
        ev(
            "selfChanged",
            PresenceEvent::SelfChanged {
                public_key: pk('f'),
                snapshot: playing(),
            },
        ),
    ]
}
