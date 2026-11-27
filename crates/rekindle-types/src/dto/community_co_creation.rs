//! Held co-creator invitations and the state of a pending creation.

use serde::{Deserialize, Serialize};

/// A friend's co-creator invite we hold, unanswered or accepted and
/// waiting for its join link (plan E3.2 S7s.3g, lead decision (33)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldCreatorInvite {
    pub anchor: String,
    /// The creator, by identity key.
    pub peer_key: String,
    pub community_name: String,
}

/// One co-creator's state in a creation of ours, in the words of
/// `FriendEvent::CoCreatorStatus` (`pending`, `accepted`, `declined`,
/// `removed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoCreatorState {
    pub peer_key: String,
    pub status: String,
}

/// A creation of ours still waiting on its co-creators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingCoCreation {
    pub anchor: String,
    pub name: String,
    /// Every co-creator named, in the order named.
    pub co_creators: Vec<CoCreatorState>,
}

/// The co-creation state a window reads once its event stream is live
/// (plan E3.2 S7s.3g, the VI1 pattern): the held invites and our pending
/// creations. The friend events then keep it current.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoCreationSnapshot {
    pub invites: Vec<HeldCreatorInvite>,
    pub creations: Vec<PendingCoCreation>,
}
