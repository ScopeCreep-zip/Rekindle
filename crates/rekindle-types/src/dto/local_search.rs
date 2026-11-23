//! Local search (plan E7.2 G11, SK2-G design §8): `IpcRequest::LocalSearch`
//! searches only what the user already holds, answered from the identity's
//! database. There is no user directory to query (threat model §4), so no
//! scope ever reaches the network.

use serde::{Deserialize, Serialize};

use super::transfer::TransferTarget;
use crate::search::SearchHit;

/// What a local search looks through: the view it was opened from (the
/// skin's `SearchFor`, design §8 (3)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    content = "arg"
)]
pub enum LocalSearchScope {
    /// Our friends, our group DMs and the community members whose cards we
    /// hold (the buddy list).
    Friends,
    /// Our favourite and recent game servers (the Servers view).
    Servers,
    /// The attachments we hold (the Downloads view).
    Files,
    /// The messages of one direct conversation: a friend's key or a group
    /// DM's id.
    Conversation { conversation_id: String },
}

/// One local search hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    content = "arg"
)]
pub enum LocalHit {
    /// An accepted friend; `name` is our nickname for them, else their
    /// display name, else empty.
    Friend { public_key: String, name: String },
    /// A group DM we are in.
    GroupDm { group_id: String, name: String },
    /// A community member by their verified card's name.
    Member {
        community: String,
        pseudonym: String,
        name: String,
    },
    /// A favourite or recent server; a recent one's `label` is empty.
    Server {
        game_id: u32,
        address: String,
        label: String,
    },
    /// A held attachment (a transfer), by the name it saves as.
    File {
        attachment_id: String,
        target: TransferTarget,
        name: String,
    },
    /// A message of the searched conversation.
    Message(SearchHit),
}

/// `LocalSearch`'s answer: friends first, then groups, members, servers,
/// files and messages, each kind in its own order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSearchResult {
    pub hits: Vec<LocalHit>,
}
