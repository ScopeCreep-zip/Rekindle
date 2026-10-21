//! The group-DM requests' `Debug` (plan E2.6): a group's name shows its
//! length only; ids and member keys are not secret.

use super::super::IpcRequest;

impl IpcRequest {
    /// The group-DM requests' `Debug`.
    pub(super) fn fmt_group_dm(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GroupDmCreate { name, members } => f
                .debug_struct("GroupDmCreate")
                .field("name_len", &name.chars().count())
                .field("members", members)
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
            other => unreachable!("{} is not a group-DM request", other.name()),
        }
    }
}
