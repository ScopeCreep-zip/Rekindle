//! Phase 21 REDO — adapter-side type-shape mapping helpers.
//!
//! Bridges the crate's veilid-free DTOs (`UserStatusKind`,
//! `GameInfoSnapshot`, `FriendPresenceEvent`) to the src-tauri
//! channel + state shapes (`UserStatus`, `GameInfoState`,
//! `crate::channels::PresenceEvent`). Sole consumer is
//! `friend_deps.rs`; lifted here so the per-trait impl files stay
//! under the 500-LoC cap and focused on one trait.

use rekindle_presence::{FriendPresenceEvent, GameInfoSnapshot, UserStatusKind};

use crate::channels::PresenceEvent;
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

pub(super) fn map_event(event: FriendPresenceEvent) -> PresenceEvent {
    match event {
        FriendPresenceEvent::FriendOnline { friend_key } => PresenceEvent::FriendOnline {
            public_key: friend_key,
        },
        FriendPresenceEvent::FriendOffline { friend_key } => PresenceEvent::FriendOffline {
            public_key: friend_key,
        },
        FriendPresenceEvent::StatusChanged { friend_key, status } => {
            PresenceEvent::StatusChanged {
                public_key: friend_key,
                status: status.as_wire_str().to_string(),
                status_message: None,
            }
        }
        FriendPresenceEvent::GameChanged { friend_key, game } => PresenceEvent::GameChanged {
            public_key: friend_key,
            game_name: game.as_ref().map(|g| g.game_name.clone()),
            game_id: game.as_ref().map(|g| g.game_id),
            elapsed_seconds: game.as_ref().map(|g| g.elapsed_seconds),
            server_address: game.as_ref().and_then(|g| g.server_address.clone()),
        },
    }
}
