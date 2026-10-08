//! Phase 21 REDO — adapter-side type-shape mapping helpers.
//!
//! Bridges the crate's veilid-free DTOs (`UserStatusKind`,
//! `GameInfoSnapshot`, `FriendPresenceEvent`) to the src-tauri
//! channel + state shapes (`UserStatus`, `GameInfoState`,
//! `crate::channels::PresenceEvent`). Sole consumer is
//! `friend_deps.rs`; lifted here so the per-trait impl files stay
//! under the 500-LoC cap and focused on one trait.

use rekindle_presence::{FriendPresenceEvent, GameInfoSnapshot, UserStatusKind};

use rekindle_types::subscription_events::{GameActivity, PresenceEvent, PresenceSnapshot};

use crate::state::{GameInfoState, UserStatus};

pub(super) fn to_crate_status(status: UserStatus) -> UserStatusKind {
    match status {
        UserStatus::Online => UserStatusKind::Online,
        UserStatus::Away => UserStatusKind::Away,
        UserStatus::Busy => UserStatusKind::Busy,
        UserStatus::Offline => UserStatusKind::Offline,
        UserStatus::Invisible => UserStatusKind::Invisible,
    }
}

pub(super) fn from_crate_status(status: UserStatusKind) -> UserStatus {
    match status {
        UserStatusKind::Online => UserStatus::Online,
        UserStatusKind::Away => UserStatus::Away,
        UserStatusKind::Busy => UserStatus::Busy,
        UserStatusKind::Offline => UserStatus::Offline,
        UserStatusKind::Invisible => UserStatus::Invisible,
    }
}

pub(super) fn from_crate_game_info(snapshot: GameInfoSnapshot) -> GameInfoState {
    GameInfoState {
        game_id: snapshot.game_id,
        game_name: snapshot.game_name,
        server_info: None,
        elapsed_seconds: snapshot.elapsed_seconds,
        server_address: snapshot.server_address,
    }
}

/// Project a crate-side friend presence event onto the daemon vocabulary.
///
/// Four variants collapse to one subject plus an observation. Online and
/// offline are statuses, not separate kinds of event; a game change
/// leaves `status` unobserved so it cannot overwrite a status the peer
/// never restated, and vice versa.
pub(super) fn map_event(event: FriendPresenceEvent) -> PresenceEvent {
    let (friend_key, snapshot) = match event {
        FriendPresenceEvent::FriendOnline { friend_key } => {
            (friend_key, PresenceSnapshot::status("online"))
        }
        FriendPresenceEvent::FriendOffline { friend_key } => {
            (friend_key, PresenceSnapshot::status("offline"))
        }
        FriendPresenceEvent::StatusChanged { friend_key, status } => {
            (friend_key, PresenceSnapshot::status(status.as_wire_str()))
        }
        FriendPresenceEvent::GameChanged { friend_key, game } => (
            friend_key,
            PresenceSnapshot::game(GameActivity::from_parts(
                game.as_ref().map(|g| g.game_name.clone()),
                game.as_ref().map(|g| g.game_id),
                game.as_ref().map(|g| g.elapsed_seconds),
                game.as_ref().and_then(|g| g.server_address.clone()),
            )),
        ),
    };
    PresenceEvent::FriendChanged {
        peer_key: friend_key,
        snapshot,
    }
}
