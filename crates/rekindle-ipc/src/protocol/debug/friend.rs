//! The friend, DM and session-reset requests' `Debug` (plan E2.1).

use super::super::IpcRequest;

impl IpcRequest {
    /// The friend requests' `Debug`: an invite link and a request's note
    /// show their length only.
    pub(super) fn fmt_friend(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FriendAdd {
                public_key,
                display_name,
                message,
            } => f
                .debug_struct("FriendAdd")
                .field("public_key", public_key)
                .field("display_name", display_name)
                .field("message_len", &message.len())
                .finish(),
            Self::FriendAccept {
                public_key,
                display_name,
            } => f
                .debug_struct("FriendAccept")
                .field("public_key", public_key)
                .field("display_name", display_name)
                .finish(),
            Self::FriendReject { public_key } => f
                .debug_struct("FriendReject")
                .field("public_key", public_key)
                .finish(),
            Self::FriendRemove { public_key } => f
                .debug_struct("FriendRemove")
                .field("public_key", public_key)
                .finish(),
            Self::FriendList => write!(f, "FriendList"),
            Self::FriendRequests => write!(f, "FriendRequests"),
            Self::FriendCancelRequest { public_key }
            | Self::FriendUnblock { public_key }
            | Self::SessionResetStart { public_key }
            | Self::SessionResetAccept { public_key }
            | Self::SessionResetDecline { public_key } => f
                .debug_struct(self.name())
                .field("public_key", public_key)
                .finish(),
            // The link carries our peer's route and records: its length only.
            Self::FriendAddFromInvite { invite } => f
                .debug_struct("FriendAddFromInvite")
                .field("invite_len", &invite.len())
                .finish(),
            Self::FriendInviteCreate => write!(f, "FriendInviteCreate"),
            Self::FriendInviteList => write!(f, "FriendInviteList"),
            Self::FriendInviteCancel { invite_id } => f
                .debug_struct("FriendInviteCancel")
                .field("invite_id", invite_id)
                .finish(),
            Self::FriendBlock {
                public_key,
                display_name,
            } => f
                .debug_struct("FriendBlock")
                .field("public_key", public_key)
                .field("display_name", display_name)
                .finish(),
            Self::FriendBlockedList => write!(f, "FriendBlockedList"),
            Self::FriendSetNickname {
                public_key,
                nickname,
            } => f
                .debug_struct("FriendSetNickname")
                .field("public_key", public_key)
                .field("nickname", nickname)
                .finish(),
            Self::FriendGroupCreate { name } => f
                .debug_struct("FriendGroupCreate")
                .field("name", name)
                .finish(),
            Self::FriendGroupRename { group_id, name } => f
                .debug_struct("FriendGroupRename")
                .field("group_id", group_id)
                .field("name", name)
                .finish(),
            Self::FriendGroupMove {
                public_key,
                group_id,
            } => f
                .debug_struct("FriendGroupMove")
                .field("public_key", public_key)
                .field("group_id", group_id)
                .finish(),
            Self::FriendGroupDelete { group_id } => f
                .debug_struct("FriendGroupDelete")
                .field("group_id", group_id)
                .finish(),
            other => write!(f, "{}", other.name()),
        }
    }
}
