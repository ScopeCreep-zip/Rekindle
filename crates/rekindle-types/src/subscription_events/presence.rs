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

    /// Build from the optional-name shape most call sites hold.
    #[must_use]
    pub fn from_parts(
        game_name: Option<String>,
        game_id: Option<u32>,
        elapsed_seconds: Option<u32>,
        server_address: Option<String>,
    ) -> Self {
        match game_name {
            Some(game_name) => Self::Playing {
                game_name,
                game_id,
                elapsed_seconds,
                server_address,
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
    /// Triggered by: gossip `PresenceUpdate`.
    CommunityMemberChanged {
        community: String,
        pseudonym: String,
        snapshot: PresenceSnapshot,
    },

    /// A DM peer's presence changed.
    /// Triggered by: `DmPayload::PresenceUpdate`.
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
}

impl PresenceEvent {
    /// The observation carried by any variant.
    #[must_use]
    pub fn snapshot(&self) -> &PresenceSnapshot {
        match self {
            Self::CommunityMemberChanged { snapshot, .. }
            | Self::FriendChanged { snapshot, .. }
            | Self::SelfChanged { snapshot, .. } => snapshot,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_is_distinguishable_from_unobserved() {
        let unobserved = PresenceSnapshot::status("online");
        let stopped = PresenceSnapshot::status("online").with_game(GameActivity::Idle);

        assert!(unobserved.game.is_none(), "no game observation was made");
        assert_eq!(stopped.game, Some(GameActivity::Idle), "observed, stopped");
        // Both report no game name — the difference is whether anyone looked.
        assert_eq!(unobserved.game_name(), None);
        assert_eq!(stopped.game_name(), None);
    }

    #[test]
    fn from_parts_round_trips_a_running_game() {
        let g = GameActivity::from_parts(
            Some("Battlefield 2".into()),
            Some(7),
            Some(1234),
            Some("10.0.0.1:16567".into()),
        );
        assert_eq!(g.game_name(), Some("Battlefield 2"));
        assert_eq!(g.game_id(), Some(7));
        assert_eq!(
            GameActivity::from_parts(None, Some(7), None, None),
            GameActivity::Idle
        );
    }

    /// The daemon IPC is postcard, which cannot decode `flatten` or a
    /// `tag = "..."` enum. Those fail at *runtime*, so only a real
    /// round trip catches them — a compile pass proves nothing here.
    /// Pins the exact JSON `src/ipc/channels/presence_events.ts` parses.
    /// The webview is the only JSON consumer of this enum, so a silent
    /// rename here is a silent frontend break — the TS types are written
    /// against these literals.
    #[test]
    fn json_shape_is_what_the_webview_parses() {
        let playing = PresenceEvent::SelfChanged {
            public_key: "abc".into(),
            snapshot: PresenceSnapshot::game(GameActivity::Playing {
                game_name: "BF2".into(),
                game_id: Some(7),
                elapsed_seconds: Some(12),
                server_address: None,
            }),
        };
        assert_eq!(
            serde_json::to_string(&playing).unwrap(),
            r#"{"selfChanged":{"publicKey":"abc","snapshot":{"status":null,"statusMessage":null,"game":{"playing":{"gameName":"BF2","gameId":7,"elapsedSeconds":12,"serverAddress":null}}}}}"#
        );

        let member = PresenceEvent::CommunityMemberChanged {
            community: "c".into(),
            pseudonym: "p".into(),
            snapshot: PresenceSnapshot::status("away"),
        };
        assert_eq!(
            serde_json::to_string(&member).unwrap(),
            r#"{"communityMemberChanged":{"community":"c","pseudonym":"p","snapshot":{"status":"away","statusMessage":null,"game":null}}}"#
        );

        // A unit variant is a bare string, not an object — the TS union
        // has to accept `"idle"` alongside `{ playing: {...} }`.
        assert_eq!(
            serde_json::to_string(&PresenceSnapshot::game(GameActivity::Idle)).unwrap(),
            r#"{"status":null,"statusMessage":null,"game":"idle"}"#
        );
    }

    #[test]
    fn postcard_round_trips_every_variant() {
        let full = PresenceSnapshot::status("online")
            .with_status_message(Some("brb".into()))
            .with_game(GameActivity::Playing {
                game_name: "Battlefield 2".into(),
                game_id: Some(7),
                elapsed_seconds: Some(1234),
                server_address: Some("10.0.0.1:16567".into()),
            });

        for event in [
            PresenceEvent::CommunityMemberChanged {
                community: "c".into(),
                pseudonym: "p".into(),
                snapshot: full.clone(),
            },
            PresenceEvent::FriendChanged {
                peer_key: "k".into(),
                snapshot: PresenceSnapshot::game(GameActivity::Idle),
            },
            PresenceEvent::SelfChanged {
                public_key: "me".into(),
                snapshot: PresenceSnapshot::default(),
            },
        ] {
            let bytes = postcard::to_allocvec(&event).expect("postcard encode");
            let back: PresenceEvent = postcard::from_bytes(&bytes).expect("postcard decode");
            assert_eq!(
                format!("{event:?}"),
                format!("{back:?}"),
                "postcard round trip must preserve the event exactly"
            );
        }
    }

    #[test]
    fn every_variant_exposes_its_snapshot() {
        let snap = PresenceSnapshot::status("away").with_status_message(Some("brb".into()));
        for event in [
            PresenceEvent::CommunityMemberChanged {
                community: "c".into(),
                pseudonym: "p".into(),
                snapshot: snap.clone(),
            },
            PresenceEvent::FriendChanged {
                peer_key: "k".into(),
                snapshot: snap.clone(),
            },
            PresenceEvent::SelfChanged {
                public_key: "me".into(),
                snapshot: snap.clone(),
            },
        ] {
            assert_eq!(event.snapshot().status.as_deref(), Some("away"));
            assert_eq!(event.snapshot().status_message.as_deref(), Some("brb"));
        }
    }
}
