//! Presence events — member status, game activity, away state.
//!
//! Presence is semi-ephemeral — entries expire after 5 minutes without
//! update. The subscription module auto-marks expired entries as offline.
//!
//! ## Why one snapshot rather than a variant per field
//!
//! This enum used to carry its fields inline, and each variant carried
//! a different subset: `CommunityMemberChanged` had `game_id`,
//! `FriendChanged` did not, and neither had `status_message`,
//! `elapsed_seconds` or `server_address`. The desktop's parallel
//! `channels::PresenceEvent` carried all of them.
//!
//! That is the same drift that split the friend list's encoders and let
//! `profile_dht_key` fall off every write: two shapes for one fact,
//! each losing a different part of it. [`PresenceSnapshot`] gives all
//! three subjects one payload, so a field can only be added in a place
//! where every subject gets it.
//!
//! Every field is `Option` because an emitter reports **what it
//! observed**, not the whole world — the game scanner knows the game
//! and nothing about status; the idle timer knows status and nothing
//! about the game. `None` means "not observed", and consumers merge
//! rather than replace. [`GameActivity`] exists so that "not observed"
//! stays distinguishable from "observed, and they stopped playing"
//! without an `Option<Option<_>>`.

//! ## Wire constraints
//!
//! `SubscriptionEvent` crosses the daemon IPC socket as **postcard**
//! (`rekindle-node/src/ipc/framing.rs`), which is not self-describing.
//! That rules out `#[serde(flatten)]` and internally/adjacently tagged
//! enums (`tag = "..."`), both of which need a self-describing format to
//! decode. So the snapshot is a nested field rather than flattened, and
//! every enum here stays externally tagged. `postcard_round_trips_every_variant`
//! is the guard: it fails at runtime, not compile time, if that is undone.

use serde::{Deserialize, Serialize};

/// What a peer is playing, when the emitter actually looked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum GameActivity {
    /// Observed, and not playing anything.
    Idle,
    /// Observed, and playing this.
    Playing {
        game_name: String,
        game_id: Option<u32>,
        /// Seconds elapsed in the current session, when known.
        elapsed_seconds: Option<u32>,
        /// Direct server address ("ip:port") for join-game, when the
        /// game exposes one.
        server_address: Option<String>,
        /// The game's icon, `src/assets/game-icons/<key>.png`, resolved by
        /// the backend from the game database (plan E7.2); never on the
        /// wire between peers.
        icon_key: Option<String>,
        /// Whether "Join Game" can start this game on their server here.
        /// The Host states it once, when it journals the event (plan E7.15
        /// IV3, decision 51); an emitter leaves it unstated (`false`, no
        /// reason). Never on the wire between peers.
        joinable: bool,
        /// Why not, once the Host has stated `joinable` as false.
        joinable_reason: Option<crate::friend::NotJoinableReason>,
    },
}

impl GameActivity {
    /// The game's name, or `None` when idle.
    #[must_use]
    pub fn game_name(&self) -> Option<&str> {
        match self {
            Self::Idle => None,
            Self::Playing { game_name, .. } => Some(game_name.as_str()),
        }
    }

    /// The game's id, or `None` when idle or unknown.
    #[must_use]
    pub fn game_id(&self) -> Option<u32> {
        match self {
            Self::Idle => None,
            Self::Playing { game_id, .. } => *game_id,
        }
    }

    /// Build from the optional-name shape most call sites hold; `joinable`
    /// stays unstated until the Host journals the event.
    #[must_use]
    pub fn from_parts(
        game_name: Option<String>,
        game_id: Option<u32>,
        elapsed_seconds: Option<u32>,
        server_address: Option<String>,
        icon_key: Option<String>,
    ) -> Self {
        match game_name {
            Some(game_name) => Self::Playing {
                game_name,
                game_id,
                elapsed_seconds,
                server_address,
                icon_key,
                joinable: false,
                joinable_reason: None,
            },
            None => Self::Idle,
        }
    }
}

/// One observation of a peer's presence.
///
/// See the module docs: `None` is "not observed", never "cleared".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceSnapshot {
    /// Wire status string — "online" / "away" / "busy" / "offline".
    pub status: Option<String>,

    /// Free-text status message accompanying `status`.
    pub status_message: Option<String>,

    /// Game activity, when the emitter looked at it.
    pub game: Option<GameActivity>,

    /// What the host makes of our input activity (`SelfChanged` only).
    pub idle: Option<crate::presence::IdleState>,
}

impl PresenceSnapshot {
    /// A status-only observation.
    #[must_use]
    pub fn status(status: impl Into<String>) -> Self {
        Self {
            status: Some(status.into()),
            ..Self::default()
        }
    }

    /// A game-only observation.
    #[must_use]
    pub fn game(game: GameActivity) -> Self {
        Self {
            game: Some(game),
            ..Self::default()
        }
    }

    /// Attach a status message to a status observation.
    #[must_use]
    pub fn with_status_message(mut self, message: Option<String>) -> Self {
        self.status_message = message;
        self
    }

    /// Attach an idle observation.
    #[must_use]
    pub fn with_idle(mut self, idle: crate::presence::IdleState) -> Self {
        self.idle = Some(idle);
        self
    }

    /// Attach a game observation.
    #[must_use]
    pub fn with_game(mut self, game: GameActivity) -> Self {
        self.game = Some(game);
        self
    }

    /// The observed game name, if a game was observed and is running.
    #[must_use]
    pub fn game_name(&self) -> Option<&str> {
        self.game.as_ref().and_then(GameActivity::game_name)
    }

    /// The observed game id, if a game was observed and is running.
    #[must_use]
    pub fn game_id(&self) -> Option<u32> {
        self.game.as_ref().and_then(GameActivity::game_id)
    }
}

/// Presence change events.
///
/// The shared `Changed` postfix is the point: each variant names *whose*
/// presence changed, and the verb is the one thing they have in common.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
#[allow(clippy::enum_variant_names, reason = "the postfix is the shared verb")]
pub enum PresenceEvent {
    /// A community member's presence changed.
    /// Triggered by: the member's beat (`M`/1, status) and card (`M`/0,
    /// game), plan E3.4 S2, S6.
    CommunityMemberChanged {
        community: String,
        pseudonym: String,
        snapshot: PresenceSnapshot,
    },

    /// A DM peer's presence changed.
    /// Triggered by: a `ValueChange` on the friend's profile record.
    FriendChanged {
        peer_key: String,
        snapshot: PresenceSnapshot,
    },

    /// **Our own** presence changed — status, or the game we are playing.
    ///
    /// The vocabulary had no way to say this: the other two variants
    /// both describe someone *else*, so the desktop's game publisher and
    /// idle timer had nowhere to land and kept using a parallel enum.
    SelfChanged {
        /// Our own identity public key, hex.
        public_key: String,
        snapshot: PresenceSnapshot,
    },

    /// **Our own** profile changed: the name we show, or our avatar (plan
    /// E1.1). Windows re-read the avatar with `AvatarGet`.
    SelfProfileChanged {
        /// Our own identity public key, hex.
        public_key: String,
        /// The new display name, when it changed.
        display_name: Option<String>,
        avatar_changed: bool,
    },
}

impl PresenceEvent {
    /// The presence observation a variant carries; `None` for a profile
    /// change.
    #[must_use]
    pub fn snapshot(&self) -> Option<&PresenceSnapshot> {
        match self {
            Self::CommunityMemberChanged { snapshot, .. }
            | Self::FriendChanged { snapshot, .. }
            | Self::SelfChanged { snapshot, .. } => Some(snapshot),
            Self::SelfProfileChanged { .. } => None,
        }
    }

    /// The presence observation a variant carries, to state what only the
    /// Host knows (`joinable`) before it is journaled.
    pub fn snapshot_mut(&mut self) -> Option<&mut PresenceSnapshot> {
        match self {
            Self::CommunityMemberChanged { snapshot, .. }
            | Self::FriendChanged { snapshot, .. }
            | Self::SelfChanged { snapshot, .. } => Some(snapshot),
            Self::SelfProfileChanged { .. } => None,
        }
    }
}

#[cfg(test)]
#[path = "presence_tests.rs"]
mod tests;
