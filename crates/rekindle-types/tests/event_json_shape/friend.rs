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
    "displayNameChanged",
    "bioChanged",
    "groupChanged",
    "keyChanged",
    "handshakeFailed",
    "handshakeRetrying",
    "reachability",
    "identityPinMismatch",
    "identityPinRestored",
    "verificationChanged",
    "blockChanged",
    "gameInvite",
    "creatorInvited",
    "creatorInviteWithdrawn",
    "coCreatorStatus",
    "coCreationFinished",
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
        FriendEvent::DisplayNameChanged { .. } => "displayNameChanged",
        FriendEvent::BioChanged { .. } => "bioChanged",
        FriendEvent::GroupChanged { .. } => "groupChanged",
        FriendEvent::KeyChanged { .. } => "keyChanged",
        FriendEvent::HandshakeFailed { .. } => "handshakeFailed",
        FriendEvent::HandshakeRetrying { .. } => "handshakeRetrying",
        FriendEvent::Reachability { .. } => "reachability",
        FriendEvent::IdentityPinMismatch { .. } => "identityPinMismatch",
        FriendEvent::IdentityPinRestored { .. } => "identityPinRestored",
        FriendEvent::VerificationChanged { .. } => "verificationChanged",
        FriendEvent::BlockChanged { .. } => "blockChanged",
        FriendEvent::GameInvite { .. } => "gameInvite",
        FriendEvent::CreatorInvited { .. } => "creatorInvited",
        FriendEvent::CreatorInviteWithdrawn { .. } => "creatorInviteWithdrawn",
        FriendEvent::CoCreatorStatus { .. } => "coCreatorStatus",
        FriendEvent::CoCreationFinished { .. } => "coCreationFinished",
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
                display_name: Some(text("Bob")),
            },
        ),
        ev("rejected", FriendEvent::Rejected { peer_key: pk('b') }),
        ev(
            "added",
            FriendEvent::Added {
                peer_key: pk('b'),
                display_name: text("Bob"),
                friendship_state: rekindle_types::friend_store::FriendshipState::PendingOut,
                group_label: text("Friends"),
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
            "displayNameChanged",
            FriendEvent::DisplayNameChanged {
                peer_key: pk('b'),
                display_name: text("Bob"),
            },
        ),
        ev("bioChanged", FriendEvent::BioChanged { peer_key: pk('b') }),
        ev(
            "groupChanged",
            FriendEvent::GroupChanged {
                peer_key: pk('b'),
                group_id: Some(7),
                group: Some(text("Raid team")),
                group_label: text("Raid team"),
            },
        ),
        ev(
            "keyChanged",
            FriendEvent::KeyChanged {
                peer_key: pk('b'),
                new_key: pk('c'),
            },
        ),
        ev(
            "handshakeFailed",
            FriendEvent::HandshakeFailed {
                peer_key: pk('b'),
                reason: text("identity key mismatch"),
            },
        ),
        ev(
            "handshakeRetrying",
            FriendEvent::HandshakeRetrying { peer_key: pk('b') },
        ),
        ev(
            "reachability",
            FriendEvent::Reachability {
                peer_key: pk('b'),
                reachable: false,
            },
        ),
        ev(
            "identityPinMismatch",
            FriendEvent::IdentityPinMismatch {
                peer_key: pk('b'),
                display_name: text("Bob"),
            },
        ),
        ev(
            "identityPinRestored",
            FriendEvent::IdentityPinRestored { peer_key: pk('b') },
        ),
        ev(
            "verificationChanged",
            FriendEvent::VerificationChanged {
                peer_key: pk('b'),
                verified: true,
            },
        ),
        ev(
            "blockChanged",
            FriendEvent::BlockChanged {
                peer_key: pk('b'),
                blocked: false,
            },
        ),
        ev(
            "gameInvite",
            FriendEvent::GameInvite {
                peer_key: pk('b'),
                game_id: 4181,
                server: rekindle_types::game_server::ServerAddress::parse("203.0.113.7:27015")
                    .unwrap(),
            },
        ),
        ev(
            "creatorInvited",
            FriendEvent::CreatorInvited {
                peer_key: pk('b'),
                anchor: rk('a'),
                community_name: text("LAN Party"),
            },
        ),
        ev(
            "creatorInviteWithdrawn",
            FriendEvent::CreatorInviteWithdrawn {
                peer_key: pk('b'),
                anchor: rk('a'),
            },
        ),
        ev(
            "coCreatorStatus",
            FriendEvent::CoCreatorStatus {
                peer_key: pk('b'),
                anchor: rk('a'),
                status: text("accepted"),
            },
        ),
        ev(
            "coCreationFinished",
            FriendEvent::CoCreationFinished {
                anchor: rk('a'),
                created: true,
            },
        ),
    ]
}
