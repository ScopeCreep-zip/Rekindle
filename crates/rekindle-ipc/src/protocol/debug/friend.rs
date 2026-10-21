//! The friend, DM, session-reset and verification requests' `Debug`
//! (plans E2.1, E2.3).

use super::super::IpcRequest;

impl IpcRequest {
    /// The friend requests' `Debug`: an invite link and a request's note
    /// show their length only.
    pub(super) fn fmt_friend(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FriendAddFromCode { code, message } => f
                .debug_struct("FriendAddFromCode")
                .field("code_len", &code.len())
                .field("message_len", &message.len())
                .finish(),
            Self::FriendRemove { public_key } => f
                .debug_struct("FriendRemove")
                .field("public_key", public_key)
                .finish(),
            Self::FriendList => write!(f, "FriendList"),
            Self::FriendRequests => write!(f, "FriendRequests"),
            Self::FriendAccept { public_key }
            | Self::FriendBlock { public_key }
            | Self::FriendReject { public_key }
            | Self::FriendCancelRequest { public_key }
            | Self::FriendRetryHandshake { public_key }
            | Self::FriendUnblock { public_key }
            | Self::SessionResetStart { public_key }
            | Self::SessionReinitiate { public_key }
            | Self::SessionResetAccept { public_key }
            | Self::IdentityRotationAccept { public_key }
            | Self::IdentityRotationDecline { public_key }
            | Self::SessionResetDecline { public_key } => f
                .debug_struct(self.name())
                .field("public_key", public_key)
                .finish(),
            Self::IdentitySafetyNumber { peer }
            | Self::IdentityMarkVerified { peer }
            | Self::IdentityClearVerification { peer } => {
                f.debug_struct(self.name()).field("peer", peer).finish()
            }
            // The link carries our peer's route and records: its length only.
            Self::FriendAddFromInvite { invite } => f
                .debug_struct("FriendAddFromInvite")
                .field("invite_len", &invite.len())
                .finish(),
            Self::FriendInviteCreate => write!(f, "FriendInviteCreate"),
            Self::FriendCode => write!(f, "FriendCode"),
            Self::FriendTokenRotate => write!(f, "FriendTokenRotate"),
            Self::FriendInviteList => write!(f, "FriendInviteList"),
            Self::FriendInviteCancel { invite_id } => f
                .debug_struct("FriendInviteCancel")
                .field("invite_id", invite_id)
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
