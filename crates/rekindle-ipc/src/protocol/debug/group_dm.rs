//! The group-DM requests' `Debug` (plan E2.6): a group's name shows its
//! length only, as a message body does; ids and member keys are not
//! secret.

use super::super::IpcRequest;

impl IpcRequest {
    /// The group-DM requests' `Debug`.
    pub(super) fn fmt_group_dm(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GroupDmCreate {
                name,
                members,
                origin_peer,
            } => f
                .debug_struct("GroupDmCreate")
                .field("name_len", &name.chars().count())
                .field("members", members)
                .field("origin_peer", &origin_peer.as_ref().map(|_| "<redacted>"))
                .finish(),
            Self::GroupDmAccept { group_id } => f
                .debug_struct("GroupDmAccept")
                .field("group_id", group_id)
                .finish(),
            Self::GroupDmDecline { group_id } => f
                .debug_struct("GroupDmDecline")
                .field("group_id", group_id)
                .finish(),
            Self::GroupDmLeave { group_id } => f
                .debug_struct("GroupDmLeave")
                .field("group_id", group_id)
                .finish(),
            Self::GroupDmRename { group_id, name } => f
                .debug_struct("GroupDmRename")
                .field("group_id", group_id)
                .field("name_len", &name.chars().count())
                .finish(),
            Self::DisappearingSet {
                conversation,
                seconds,
            } => f
                .debug_struct("DisappearingSet")
                .field("conversation", conversation)
                .field("seconds", seconds)
                .finish(),
            Self::DisappearingGet { conversation } => f
                .debug_struct("DisappearingGet")
                .field("conversation", conversation)
                .finish(),
            Self::GroupDmSend { group_id, body } => f
                .debug_struct("GroupDmSend")
                .field("group_id", group_id)
                .field("body_len", &body.len())
                .finish(),
            Self::GroupDmTyping { group_id, typing } => f
                .debug_struct("GroupDmTyping")
                .field("group_id", group_id)
                .field("typing", typing)
                .finish(),
            Self::GroupDmList => f.write_str("GroupDmList"),
            Self::GroupDmMessages {
                group_id,
                limit,
                before,
            } => f
                .debug_struct("GroupDmMessages")
                .field("group_id", group_id)
                .field("limit", limit)
                .field("before", before)
                .finish(),
            Self::GroupDmMarkRead { group_id } => f
                .debug_struct("GroupDmMarkRead")
                .field("group_id", group_id)
                .finish(),
            other => unreachable!("{} is not a group-DM request", other.name()),
        }
    }
}
