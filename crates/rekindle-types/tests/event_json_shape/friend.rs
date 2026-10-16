//! `FriendEvent` samples.

use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use crate::{pk, rk, text, Sample};

pub const NAMES: &[&str] = &[
    "requestReceived",
    "requestAcknowledged",
    "accepted",
    "rejected",
    "added",
    "removed",
    "removeAcknowledged",
    "profileKeyRotated",
    "nicknameChanged",
    "handshakeFailed",
];

pub fn variant_name(e: &FriendEvent) -> &'static str {
    match e {
        FriendEvent::RequestReceived { .. } => "requestReceived",
        FriendEvent::RequestAcknowledged { .. } => "requestAcknowledged",
        FriendEvent::Accepted { .. } => "accepted",
        FriendEvent::Rejected { .. } => "rejected",
        FriendEvent::Added { .. } => "added",
        FriendEvent::Removed { .. } => "removed",
        FriendEvent::RemoveAcknowledged { .. } => "removeAcknowledged",
        FriendEvent::ProfileKeyRotated { .. } => "profileKeyRotated",
        FriendEvent::NicknameChanged { .. } => "nicknameChanged",
        FriendEvent::HandshakeFailed { .. } => "handshakeFailed",
    }
}

fn ev(name: &str, e: FriendEvent) -> Sample {
    (format!("friend.{name}"), SubscriptionEvent::Friend(e))
}

pub fn samples() -> Vec<Sample> {
    vec![
        ev(
            "requestReceived",
            FriendEvent::RequestReceived {
                from_key: pk('b'),
                display_name: text("Bob"),
                message: text("remember me from LAN night?"),
            },
        ),
        ev(
            "requestAcknowledged",
            FriendEvent::RequestAcknowledged { peer_key: pk('b') },
        ),
        ev(
            "accepted",
            FriendEvent::Accepted {
                peer_key: pk('b'),
                dm_log_key: Some(rk('l')),
                display_name: Some(text("Bob")),
            },
        ),
        ev("rejected", FriendEvent::Rejected { peer_key: pk('b') }),
        ev(
            "added",
            FriendEvent::Added {
                peer_key: pk('b'),
                display_name: text("Bob"),
                friendship_state: text("active"),
            },
        ),
        ev("removed", FriendEvent::Removed { peer_key: pk('b') }),
        ev(
            "removeAcknowledged",
            FriendEvent::RemoveAcknowledged { peer_key: pk('b') },
        ),
        ev(
            "profileKeyRotated",
            FriendEvent::ProfileKeyRotated {
                peer_key: pk('b'),
                new_profile_dht_key: rk('p'),
            },
        ),
        ev(
            "nicknameChanged",
            FriendEvent::NicknameChanged {
                peer_key: pk('b'),
                nickname: Some(text("Bobby")),
            },
        ),
        ev(
            "handshakeFailed",
            FriendEvent::HandshakeFailed {
                peer_key: pk('b'),
                reason: text("identity key mismatch"),
            },
        ),
    ]
}
