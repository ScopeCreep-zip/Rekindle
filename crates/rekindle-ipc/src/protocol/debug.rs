//! Custom `Debug` impl for `IpcRequest` that redacts secret-bearing fields.

use super::IpcRequest;

mod friend;

const REDACTED: &str = "***REDACTED***";

/// [RC-16] Custom Debug that redacts every passphrase and archive.
/// All other variants display their fields normally. Body content is shown
/// as length only to avoid logging message text.
impl std::fmt::Debug for IpcRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Secret-bearing variants — redact
            Self::Unlock { identity, .. } => f
                .debug_struct("Unlock")
                .field("identity", identity)
                .field("passphrase", &REDACTED)
                .finish(),
            Self::IdentityCreate { display_name, .. } => f
                .debug_struct("IdentityCreate")
                .field("display_name", display_name)
                .field("passphrase", &REDACTED)
                .finish(),
            Self::IdentityExport { .. } => f
                .debug_struct("IdentityExport")
                .field("passphrase", &REDACTED)
                .finish(),
            Self::IdentityImport { archive, .. } => f
                .debug_struct("IdentityImport")
                .field("archive_len", &archive.len())
                .field("passphrase", &REDACTED)
                .finish(),
            Self::IdentityRotate { .. } => f
                .debug_struct("IdentityRotate")
                .field("passphrase", &REDACTED)
                .finish(),

            // Variants with message bodies — show length only
            Self::ChannelSend {
                community,
                channel,
                body,
                reply_to,
            } => f
                .debug_struct("ChannelSend")
                .field("community", community)
                .field("channel", channel)
                .field("body_len", &body.len())
                .field("reply_to", reply_to)
                .finish(),
            Self::DmSend {
                peer_key,
                body,
                reply_to,
            } => f
                .debug_struct("DmSend")
                .field("peer_key", peer_key)
                .field("body_len", &body.len())
                .field("reply_to", reply_to)
                .finish(),

            // Everything else — derive-style display
            Self::Lock => write!(f, "Lock"),
            Self::Logout => write!(f, "Logout"),
            Self::Status => write!(f, "Status"),
            Self::IdentityList => write!(f, "IdentityList"),
            Self::IdentityShow => write!(f, "IdentityShow"),
            Self::PrekeyStatus => write!(f, "PrekeyStatus"),
            Self::IdentityDelete { identity } => f
                .debug_struct("IdentityDelete")
                .field("identity", identity)
                .finish(),
            Self::IdentityWipe => f.write_str("IdentityWipe"),
            // `IntentProof`'s own `Debug` redacts a passphrase.
            Self::IntentCommit { intent_id, proof } => f
                .debug_struct("IntentCommit")
                .field("intent_id", intent_id)
                .field("proof", proof)
                .finish(),
            Self::IntentCancel { intent_id } => f
                .debug_struct("IntentCancel")
                .field("intent_id", intent_id)
                .finish(),
            Self::FriendAddFromCode { .. }
            | Self::FriendAccept { .. }
            | Self::FriendReject { .. }
            | Self::FriendRemove { .. }
            | Self::FriendList
            | Self::FriendRequests
            | Self::FriendCancelRequest { .. }
            | Self::FriendRetryHandshake { .. }
            | Self::FriendAddFromInvite { .. }
            | Self::FriendInviteCreate
            | Self::FriendCode
            | Self::FriendTokenRotate
            | Self::FriendInviteList
            | Self::FriendInviteCancel { .. }
            | Self::FriendBlock { .. }
            | Self::FriendUnblock { .. }
            | Self::FriendBlockedList
            | Self::FriendSetNickname { .. }
            | Self::FriendGroupCreate { .. }
            | Self::FriendGroupRename { .. }
            | Self::FriendGroupMove { .. }
            | Self::FriendGroupDelete { .. }
            | Self::SessionResetStart { .. }
            | Self::SessionResetAccept { .. }
            | Self::IdentityRotationAccept { .. }
            | Self::IdentityRotationDecline { .. }
            | Self::SessionResetDecline { .. }
            | Self::IdentitySafetyNumber { .. }
            | Self::IdentityMarkVerified { .. }
            | Self::IdentityClearVerification { .. } => self.fmt_friend(f),
            Self::CommunityCreate {
                name,
                description,
                approval_required,
            } => f
                .debug_struct("CommunityCreate")
                .field("name", name)
                .field("description", description)
                .field("approval_required", approval_required)
                .finish(),
            Self::CommunityJoin { invite } => f
                .debug_struct("CommunityJoin")
                .field("invite", invite)
                .finish(),
            Self::CommunityLeave { governance_key } => f
                .debug_struct("CommunityLeave")
                .field("governance_key", governance_key)
                .finish(),
            Self::CommunityList => write!(f, "CommunityList"),
            Self::CommunityInfo { governance_key } => f
                .debug_struct("CommunityInfo")
                .field("governance_key", governance_key)
                .finish(),
            Self::CommunityApprove {
                governance_key,
                member_pseudonym,
            } => f
                .debug_struct("CommunityApprove")
                .field("governance_key", governance_key)
                .field("member_pseudonym", member_pseudonym)
                .finish(),
            Self::CommunityReject {
                governance_key,
                member_pseudonym,
                reason,
            } => f
                .debug_struct("CommunityReject")
                .field("governance_key", governance_key)
                .field("member_pseudonym", member_pseudonym)
                .field("reason", reason)
                .finish(),
            Self::CommunityPendingMembers { governance_key } => f
                .debug_struct("CommunityPendingMembers")
                .field("governance_key", governance_key)
                .finish(),
            Self::CommunityTransferOwnership {
                governance_key,
                new_owner_pseudonym,
            } => f
                .debug_struct("CommunityTransferOwnership")
                .field("governance_key", governance_key)
                .field("new_owner_pseudonym", new_owner_pseudonym)
                .finish(),
            Self::ChannelList { community } => f
                .debug_struct("ChannelList")
                .field("community", community)
                .finish(),
            Self::ChannelCreate {
                community,
                name,
                kind,
                category,
                topic,
                slowmode_seconds,
            } => f
                .debug_struct("ChannelCreate")
                .field("community", community)
                .field("name", name)
                .field("kind", kind)
                .field("category", category)
                .field("topic", topic)
                .field("slowmode_seconds", slowmode_seconds)
                .finish(),
            Self::ChannelDelete {
                community,
                channel_id,
            } => f
                .debug_struct("ChannelDelete")
                .field("community", community)
                .field("channel_id", channel_id)
                .finish(),
            Self::ChannelUpdate {
                community,
                channel_id,
                name,
                topic,
                slowmode_seconds,
            } => f
                .debug_struct("ChannelUpdate")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("name", name)
                .field("topic", topic)
                .field("slowmode_seconds", slowmode_seconds)
                .finish(),
            Self::ChannelHistory {
                community,
                channel,
                limit,
            } => f
                .debug_struct("ChannelHistory")
                .field("community", community)
                .field("channel", channel)
                .field("limit", limit)
                .finish(),
            Self::DmTyping { peer_key, typing } => f
                .debug_struct("DmTyping")
                .field("peer_key", peer_key)
                .field("typing", typing)
                .finish(),
            Self::DmInbox { limit } => f.debug_struct("DmInbox").field("limit", limit).finish(),
            Self::DmHistory {
                peer_key,
                limit,
                before,
            } => f
                .debug_struct("DmHistory")
                .field("peer_key", peer_key)
                .field("limit", limit)
                .field("before", before)
                .finish(),
            Self::DmMarkRead { peer_key } => f
                .debug_struct("DmMarkRead")
                .field("peer_key", peer_key)
                .finish(),
            Self::DmResend { message_id } => f
                .debug_struct("DmResend")
                .field("message_id", message_id)
                .finish(),
            Self::Subscribe { filters, from } => f
                .debug_struct("Subscribe")
                .field("filter_count", &filters.len())
                .field("from", from)
                .finish(),
            Self::Unsubscribe { filters } => f
                .debug_struct("Unsubscribe")
                .field("filter_count", &filters.len())
                .finish(),
            Self::MekList { community } => f
                .debug_struct("MekList")
                .field("community", community)
                .finish(),
            Self::MekRotate { community, channel } => f
                .debug_struct("MekRotate")
                .field("community", community)
                .field("channel", channel)
                .finish(),
            Self::MekRequest {
                community,
                channel,
                generation,
            } => f
                .debug_struct("MekRequest")
                .field("community", community)
                .field("channel", channel)
                .field("generation", generation)
                .finish(),

            // The remaining variants are formatted by `fmt_rest` to keep this
            // method under the clippy::too_many_lines limit. They are listed
            // explicitly rather than via `_` so adding an `IpcRequest` variant
            // remains a compile error here — exhaustiveness is preserved.
            Self::Undo { .. }
            | Self::PresenceSetMessage { .. }
            | Self::PresenceGet
            | Self::ProfileSetNickname { .. }
            | Self::ProfileSetAvatar { .. }
            | Self::AvatarGet { .. }
            | Self::AuditVerify
            | Self::AuditExport { .. }
            | Self::LinkOpen { .. } => self.fmt_own(f),
            Self::PresenceSet { .. }
            | Self::GamePresenceSet { .. }
            | Self::GamePresenceClear
            | Self::RoleList { .. }
            | Self::RoleCreate { .. }
            | Self::RoleUpdate { .. }
            | Self::RoleDelete { .. }
            | Self::RoleAssign { .. }
            | Self::RoleUnassign { .. }
            | Self::Kick { .. }
            | Self::Ban { .. }
            | Self::Unban { .. }
            | Self::Timeout { .. }
            | Self::BanList { .. }
            | Self::InviteCreate { .. }
            | Self::InviteList { .. }
            | Self::InviteRevoke { .. }
            | Self::VoiceJoin { .. }
            | Self::VoiceLeave
            | Self::NetworkStatus
            | Self::NetworkPeers
            | Self::AgentRegister { .. }
            | Self::AgentRevoke { .. }
            | Self::PolicyReload
            | Self::Shutdown => self.fmt_rest(f),
        }
    }
}

impl IpcRequest {
    /// Our own profile, presence, audit log and links: text the user wrote
    /// and a URL that may carry a token are logged by length only.
    fn fmt_own(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PresenceSetMessage { message } => f
                .debug_struct("PresenceSetMessage")
                .field("message_len", &message.len())
                .finish(),
            Self::PresenceGet => write!(f, "PresenceGet"),
            Self::Undo { id } => f.debug_struct("Undo").field("id", id).finish(),
            Self::ProfileSetNickname { display_name } => f
                .debug_struct("ProfileSetNickname")
                .field("display_name_len", &display_name.len())
                .finish(),
            Self::ProfileSetAvatar { image, kind } => f
                .debug_struct("ProfileSetAvatar")
                .field("image_len", &image.len())
                .field("kind", kind)
                .finish(),
            Self::AvatarGet { public_key } => f
                .debug_struct("AvatarGet")
                .field("public_key", public_key)
                .finish(),
            Self::AuditVerify => write!(f, "AuditVerify"),
            Self::AuditExport { since } => {
                f.debug_struct("AuditExport").field("since", since).finish()
            }
            Self::LinkOpen { url } => f
                .debug_struct("LinkOpen")
                .field("url_len", &url.len())
                .finish(),
            other => unreachable!("{} is formatted by fmt", other.name()),
        }
    }

    /// Tail of the [`std::fmt::Debug`] impl — formats the variants not handled
    /// inline by `fmt`. See that impl for the split rationale. Variants handled
    /// by `fmt` are routed there and never reach the `unreachable!` arm below.
    fn fmt_rest(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PresenceSet { status } => f
                .debug_struct("PresenceSet")
                .field("status", status)
                .finish(),
            Self::GamePresenceSet {
                game_name,
                game_id,
                elapsed_seconds,
                server_address,
            } => f
                .debug_struct("GamePresenceSet")
                .field("game_name", game_name)
                .field("game_id", game_id)
                .field("elapsed_seconds", elapsed_seconds)
                .field("server_address", server_address)
                .finish(),
            Self::GamePresenceClear => write!(f, "GamePresenceClear"),
            Self::RoleList { community } => f
                .debug_struct("RoleList")
                .field("community", community)
                .finish(),
            Self::RoleCreate {
                community,
                name,
                permissions,
                color,
                position,
            } => f
                .debug_struct("RoleCreate")
                .field("community", community)
                .field("name", name)
                .field("permissions", permissions)
                .field("color", color)
                .field("position", position)
                .finish(),
            Self::RoleUpdate {
                community,
                role_id,
                name,
                permissions,
                color,
            } => f
                .debug_struct("RoleUpdate")
                .field("community", community)
                .field("role_id", role_id)
                .field("name", name)
                .field("permissions", permissions)
                .field("color", color)
                .finish(),
            Self::RoleDelete { community, role_id } => f
                .debug_struct("RoleDelete")
                .field("community", community)
                .field("role_id", role_id)
                .finish(),
            Self::RoleAssign {
                community,
                member_pseudonym,
                role_id,
            } => f
                .debug_struct("RoleAssign")
                .field("community", community)
                .field("member_pseudonym", member_pseudonym)
                .field("role_id", role_id)
                .finish(),
            Self::RoleUnassign {
                community,
                member_pseudonym,
                role_id,
            } => f
                .debug_struct("RoleUnassign")
                .field("community", community)
                .field("member_pseudonym", member_pseudonym)
                .field("role_id", role_id)
                .finish(),
            Self::Kick {
                community,
                target_pseudonym,
            } => f
                .debug_struct("Kick")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .finish(),
            Self::Ban {
                community,
                target_pseudonym,
                reason,
            } => f
                .debug_struct("Ban")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .field("reason", reason)
                .finish(),
            Self::Unban {
                community,
                target_pseudonym,
            } => f
                .debug_struct("Unban")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .finish(),
            Self::Timeout {
                community,
                target_pseudonym,
                duration_seconds,
                reason,
            } => f
                .debug_struct("Timeout")
                .field("community", community)
                .field("target_pseudonym", target_pseudonym)
                .field("duration_seconds", duration_seconds)
                .field("reason", reason)
                .finish(),
            Self::BanList { community } => f
                .debug_struct("BanList")
                .field("community", community)
                .finish(),
            Self::InviteCreate {
                community,
                max_uses,
                expires_seconds,
            } => f
                .debug_struct("InviteCreate")
                .field("community", community)
                .field("max_uses", max_uses)
                .field("expires_seconds", expires_seconds)
                .finish(),
            Self::InviteList { community } => f
                .debug_struct("InviteList")
                .field("community", community)
                .finish(),
            Self::InviteRevoke {
                community,
                invite_code,
            } => f
                .debug_struct("InviteRevoke")
                .field("community", community)
                .field("invite_code", invite_code)
                .finish(),
            Self::VoiceJoin {
                community,
                channel,
                muted,
                deafened,
            } => f
                .debug_struct("VoiceJoin")
                .field("community", community)
                .field("channel", channel)
                .field("muted", muted)
                .field("deafened", deafened)
                .finish(),
            Self::VoiceLeave => write!(f, "VoiceLeave"),
            Self::NetworkStatus => write!(f, "NetworkStatus"),
            Self::NetworkPeers => write!(f, "NetworkPeers"),
            Self::AgentRegister {
                name,
                agent_type,
                capabilities,
            } => f
                .debug_struct("AgentRegister")
                .field("name", name)
                .field("agent_type", agent_type)
                .field("capabilities", capabilities)
                .finish(),
            Self::AgentRevoke { name } => {
                f.debug_struct("AgentRevoke").field("name", name).finish()
            }
            Self::PolicyReload => write!(f, "PolicyReload"),
            Self::Shutdown => write!(f, "Shutdown"),

            // Unreachable: the variants above are the only ones routed here by
            // `fmt`, whose match is exhaustive — a new `IpcRequest` variant is
            // a compile error there, not a silent fall-through to this arm.
            _ => unreachable!("fmt_rest received a variant owned by the Debug impl"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_passphrase_or_archive_reaches_debug_output() {
        let secret = "correct horse battery staple";
        for request in [
            IpcRequest::Unlock {
                identity: "pk".into(),
                passphrase: secret.into(),
            },
            IpcRequest::IdentityCreate {
                display_name: Some("Name".into()),
                passphrase: secret.into(),
            },
            IpcRequest::IdentityExport {
                passphrase: secret.into(),
            },
            IpcRequest::IdentityImport {
                archive: secret.into(),
                passphrase: secret.into(),
            },
            IpcRequest::IdentityRotate {
                passphrase: secret.into(),
            },
        ] {
            let shown = format!("{request:?}");
            assert!(!shown.contains(secret), "{shown}");
            assert!(shown.contains(REDACTED), "{shown}");
        }
    }
}
