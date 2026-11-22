//! The activity feed's derivation (plan E7.15, IV1; Infoview design §2.3):
//! a pure diff from the previous to the next snapshot of a friend, plus
//! our own events, into [`ActivityEntry`]s for `rekindle_db::repo::activity`.
//!
//! Rules (design §2.3, §3):
//! - Entries come only from data the friend already sent us. A snapshot
//!   holds exactly that: their published name, status message, bio,
//!   current game (present only while they share it) and the game ids of
//!   the totals they share (absent while they do not).
//! - Only an accepted friend produces entries.
//! - A friend not sharing their current game produces no game entry; one
//!   not sharing totals produces no "first time" entry, because there is
//!   nothing to compare against.
//! - Our own nickname for a friend is not news: a snapshot never holds it.
//!
//! Pure, so the Host's ingest (IV2) and the netsim test it as is.

use std::collections::BTreeSet;

use rekindle_types::activity::{ActivityEntry, ActivityKind};
use rekindle_types::friend::GameInfoState;

/// What a friend has told us, as the feed compares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendActivitySnapshot {
    /// Their public key, hex: the entry's `friend`.
    pub public_key: String,
    /// The friendship is accepted (a pending one shows no presence).
    pub accepted: bool,
    /// The display name they publish.
    pub display_name: String,
    pub status_message: Option<String>,
    pub bio: Option<String>,
    /// Their current game; `None` when they play nothing or do not share it.
    pub game: Option<GameInfoState>,
    /// The game ids in the totals they share; `None` when they do not.
    pub played_games: Option<BTreeSet<u32>>,
}

/// The entries `next` makes news of against `prev`, all at `at` (Unix ms).
/// `prev` is `None` when they were not a friend of ours before (a new
/// friendship), and a pending snapshot reads the same.
///
/// A new friendship makes only [`ActivityKind::NowFriends`]: what they
/// were already doing is their state, not a change.
#[must_use]
pub fn friend_entries(
    prev: Option<&FriendActivitySnapshot>,
    next: &FriendActivitySnapshot,
    at: i64,
) -> Vec<ActivityEntry> {
    if !next.accepted {
        return Vec::new();
    }
    let friend = next.public_key.clone();
    let Some(prev) = prev.filter(|p| p.accepted) else {
        return vec![ActivityEntry {
            at,
            kind: ActivityKind::NowFriends { friend },
        }];
    };
    let mut kinds = Vec::new();
    if prev.display_name != next.display_name && !next.display_name.is_empty() {
        kinds.push(ActivityKind::RenamedSelf {
            friend: friend.clone(),
            from: prev.display_name.clone(),
            to: next.display_name.clone(),
        });
    }
    if let Some(text) = changed_text(prev.status_message.as_ref(), next.status_message.as_ref()) {
        kinds.push(ActivityKind::StatusMessage {
            friend: friend.clone(),
            text: text.to_string(),
        });
    }
    if changed_text(prev.bio.as_ref(), next.bio.as_ref()).is_some() {
        kinds.push(ActivityKind::BioChanged {
            friend: friend.clone(),
        });
    }
    game_entries(prev, next, &friend, &mut kinds);
    kinds
        .into_iter()
        .map(|kind| ActivityEntry { at, kind })
        .collect()
}

/// We added a favourite server.
#[must_use]
pub fn favourite_added(game_id: u32, address: &str, label: &str, at: i64) -> ActivityEntry {
    ActivityEntry {
        at,
        kind: ActivityKind::FavouriteAdded {
            game_id,
            address: address.to_string(),
            label: label.to_string(),
        },
    }
}

/// We joined `community`.
#[must_use]
pub fn joined_community(community: &str, at: i64) -> ActivityEntry {
    ActivityEntry {
        at,
        kind: ActivityKind::JoinedCommunity {
            community: community.to_string(),
        },
    }
}

/// `next` when it is set, not empty and not `prev`; a cleared text is not
/// news.
fn changed_text<'a>(prev: Option<&String>, next: Option<&'a String>) -> Option<&'a str> {
    next.filter(|n| !n.is_empty() && Some(*n) != prev)
        .map(String::as_str)
}

/// Stopped the old game, started the new one, and the new one's first
/// time when their shared totals before it did not hold it.
fn game_entries(
    prev: &FriendActivitySnapshot,
    next: &FriendActivitySnapshot,
    friend: &str,
    kinds: &mut Vec<ActivityKind>,
) {
    let (old, new) = (prev.game.as_ref(), next.game.as_ref());
    if old.map(|g| g.game_id) == new.map(|g| g.game_id) {
        return;
    }
    if let Some(old) = old {
        kinds.push(ActivityKind::StoppedGame {
            friend: friend.to_string(),
            game_id: old.game_id,
            seconds: old.elapsed_seconds,
        });
    }
    if let Some(new) = new {
        kinds.push(ActivityKind::StartedGame {
            friend: friend.to_string(),
            game_id: new.game_id,
            server: new.server_address.clone(),
        });
        let first_time = prev
            .played_games
            .as_ref()
            .is_some_and(|played| !played.contains(&new.game_id));
        if first_time {
            kinds.push(ActivityKind::FirstPlayed {
                friend: friend.to_string(),
                game_id: new.game_id,
            });
        }
    }
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
