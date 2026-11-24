//! Group DMs (plan E2.6), carried as `IpcRequest::GroupDm`: create a group
//! of us and two to seven friends, answer its invite, rename it, invite
//! more friends, send and read its messages, and leave it. The group's game
//! lobby (`IpcRequest::Lobby`) and the disappearing timer a group shares
//! with a 1:1 DM (`IpcRequest::DisappearingSet`) stay their own requests.
//! Every leaf keeps its own name, as `LobbyRequest`'s do: the variants are
//! short (`GroupDmRequest::Create`) but each is renamed on the wire, in
//! `name()` and so in every registry (`COMMAND_TABLE`, the shell's command
//! map) to its leaf name, which is unique across `IpcRequest`.

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A group-DM request.
#[derive(Clone, Serialize, Deserialize)]
pub enum GroupDmRequest {
    /// Create a group named `name` of us and `members` (2-7 friends'
    /// keys); the answer is its id. `origin_peer`: the member whose chat it
    /// was created from (SK2-G §6, the room attached to that chat).
    #[serde(rename = "GroupDmCreate")]
    Create {
        name: String,
        members: Vec<String>,
        origin_peer: Option<String>,
    },
    /// Accept the pending invite to `group_id` (32 hex).
    #[serde(rename = "GroupDmAccept")]
    Accept { group_id: String },
    /// Decline the pending invite to `group_id`.
    #[serde(rename = "GroupDmDecline")]
    Decline { group_id: String },
    /// Leave `group_id`; every other member rotates its key.
    #[serde(rename = "GroupDmLeave")]
    Leave { group_id: String },
    /// Rename `group_id` (its creator only; 1-`MAX_NAME_CHARS`
    /// characters); every member is told through our group chain.
    #[serde(rename = "GroupDmRename")]
    Rename { group_id: String, name: String },
    /// Invite friends into `group_id` (creator only, at most 8; E7.2 G8).
    #[serde(rename = "GroupDmInvite")]
    Invite {
        group_id: String,
        members: Vec<String>,
    },
    /// Send `body` to `group_id`; the answer is the message's client id
    /// once its row and local echo are committed (`queued`, then
    /// `GroupDmEvent::SendStatus`).
    #[serde(rename = "GroupDmSend")]
    Send { group_id: String, body: String },
    /// The draft to `group_id` changed: `typing` is whether it holds text.
    /// The Host decides what goes out, as for `DmTyping`.
    #[serde(rename = "GroupDmTyping")]
    Typing { group_id: String, typing: bool },
    /// Every group we are in and every invite we have not answered.
    #[serde(rename = "GroupDmList")]
    List,
    /// The newest `limit` messages of `group_id` before `before` (ms).
    #[serde(rename = "GroupDmMessages")]
    Messages {
        group_id: String,
        limit: u32,
        before: Option<i64>,
    },
    /// The group's window displayed its messages (plan E7.1): the
    /// received messages whose disappearing timer had not started start it.
    #[serde(rename = "GroupDmMarkRead")]
    MarkRead { group_id: String },
}

impl GroupDmRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::Create { .. } => ("GroupDmCreate", Write, Authenticated),
            Self::Accept { .. } => ("GroupDmAccept", Write, Authenticated),
            Self::Decline { .. } => ("GroupDmDecline", Write, Authenticated),
            Self::Leave { .. } => ("GroupDmLeave", Write, Authenticated),
            Self::Rename { .. } => ("GroupDmRename", Write, Authenticated),
            Self::Invite { .. } => ("GroupDmInvite", Write, Authenticated),
            Self::Send { .. } => ("GroupDmSend", Write, Authenticated),
            Self::Typing { .. } => ("GroupDmTyping", Write, Authenticated),
            Self::List => ("GroupDmList", Query, Authenticated),
            Self::Messages { .. } => ("GroupDmMessages", Query, Authenticated),
            Self::MarkRead { .. } => ("GroupDmMarkRead", Write, Authenticated),
        }
    }

    /// The group's list and messages are read as DM history is; everything
    /// else writes as a DM does.
    #[must_use]
    pub fn agent_capability(&self) -> AgentCapability {
        match self {
            Self::List | Self::Messages { .. } => AgentCapability::ImHistory,
            Self::Create { .. }
            | Self::Accept { .. }
            | Self::Decline { .. }
            | Self::Leave { .. }
            | Self::Rename { .. }
            | Self::Invite { .. }
            | Self::Send { .. }
            | Self::Typing { .. }
            | Self::MarkRead { .. } => AgentCapability::ImWrite,
        }
    }
}

impl std::fmt::Debug for GroupDmRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Self::List = self {
            return f.write_str("GroupDmList");
        }
        let mut d = f.debug_struct(self.name());
        match self {
            Self::List => {}
            Self::Create {
                name,
                members,
                origin_peer,
            } => {
                d.field("name_len", &name.chars().count())
                    .field("members", members)
                    .field("origin_peer", &origin_peer.as_ref().map(|_| "<redacted>"));
            }
            Self::Accept { group_id }
            | Self::Decline { group_id }
            | Self::Leave { group_id }
            | Self::MarkRead { group_id } => {
                d.field("group_id", group_id);
            }
            Self::Invite { group_id, members } => {
                d.field("group_id", group_id)
                    .field("members", &members.len());
            }
            Self::Rename { group_id, name } => {
                d.field("group_id", group_id)
                    .field("name_len", &name.chars().count());
            }
            Self::Send { group_id, body } => {
                d.field("group_id", group_id).field("body_len", &body.len());
            }
            Self::Typing { group_id, typing } => {
                d.field("group_id", group_id).field("typing", typing);
            }
            Self::Messages {
                group_id,
                limit,
                before,
            } => {
                d.field("group_id", group_id)
                    .field("limit", limit)
                    .field("before", before);
            }
        }
        d.finish()
    }
}
