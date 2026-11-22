//! The local activity feed (plan E7.15, IV1): what the Infoview's
//! Activity section lists, Rekindle's stand-in for Xfire's server-fed
//! Activity Report (`friends.tmpl`).
//!
//! Every entry is derived on this device from data a friend already sent
//! us over the ratchet (presence, game, totals, bio) or from our own
//! actions; nothing is fetched to make one (threat model: "There is no
//! server"). Entries name a friend by public key and a game by id: the
//! Host resolves the stored display name, our nickname and the game's name
//! and icon when it shows them, so a stored entry carries no spoofable
//! display field (Infoview design §3.6).

use serde::{Deserialize, Serialize};

/// One thing that happened, as the feed stores and shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type",
    content = "data"
)]
pub enum ActivityKind {
    /// A friend started playing `game_id`, on `server` when they shared
    /// one (`host:port`, parsed on ingest).
    StartedGame {
        friend: String,
        game_id: u32,
        server: Option<String>,
    },
    /// A friend stopped playing `game_id` after `seconds`.
    StoppedGame {
        friend: String,
        game_id: u32,
        seconds: u32,
    },
    /// A friend played `game_id` for the first time: it was not in the
    /// game totals they had shared before.
    FirstPlayed { friend: String, game_id: u32 },
    /// A friend changed the display name they publish. Our own nickname
    /// for them is not news and is never logged.
    RenamedSelf {
        friend: String,
        from: String,
        to: String,
    },
    /// A friend set a new status message.
    StatusMessage { friend: String, text: String },
    /// A friend changed their bio.
    BioChanged { friend: String },
    /// We and `friend` became friends.
    NowFriends { friend: String },
    /// We added a favourite server (only ours: a friend's never reaches
    /// us).
    FavouriteAdded {
        game_id: u32,
        address: String,
        label: String,
    },
    /// We joined a community.
    JoinedCommunity { community: String },
}

impl ActivityKind {
    /// The kind's name, the `activity_log.kind` column.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::StartedGame { .. } => "startedGame",
            Self::StoppedGame { .. } => "stoppedGame",
            Self::FirstPlayed { .. } => "firstPlayed",
            Self::RenamedSelf { .. } => "renamedSelf",
            Self::StatusMessage { .. } => "statusMessage",
            Self::BioChanged { .. } => "bioChanged",
            Self::NowFriends { .. } => "nowFriends",
            Self::FavouriteAdded { .. } => "favouriteAdded",
            Self::JoinedCommunity { .. } => "joinedCommunity",
        }
    }

    /// The friend the entry is about, or `None` for our own entries. The
    /// `activity_log.subject_key` column: a friend's removal or block
    /// purges by it.
    #[must_use]
    pub fn friend(&self) -> Option<&str> {
        match self {
            Self::StartedGame { friend, .. }
            | Self::StoppedGame { friend, .. }
            | Self::FirstPlayed { friend, .. }
            | Self::RenamedSelf { friend, .. }
            | Self::StatusMessage { friend, .. }
            | Self::BioChanged { friend }
            | Self::NowFriends { friend } => Some(friend),
            Self::FavouriteAdded { .. } | Self::JoinedCommunity { .. } => None,
        }
    }
}

/// A stored feed entry: when it happened (Unix ms) and what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    pub at: i64,
    pub kind: ActivityKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire shape is the IPC enum convention, and `name()` is the
    /// serde tag, so the `kind` column and the payload agree.
    #[test]
    fn the_tag_is_the_name_and_fields_are_camel_case() {
        let kind = ActivityKind::StartedGame {
            friend: "ab".into(),
            game_id: 7,
            server: Some("203.0.113.7:27015".into()),
        };
        let json = serde_json::to_value(&kind).unwrap();
        assert_eq!(json["type"], kind.name());
        assert_eq!(json["data"]["gameId"], 7);
        assert_eq!(kind.friend(), Some("ab"));
        let own = ActivityKind::JoinedCommunity {
            community: "c".into(),
        };
        assert_eq!(serde_json::to_value(&own).unwrap()["type"], own.name());
        assert_eq!(own.friend(), None);
    }
}
