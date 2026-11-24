//! Custom `Debug` impl for `IpcRequest` that redacts secret-bearing fields.

use super::IpcRequest;

mod community;
mod dm;
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
            Self::QuickWipeArm { shortcut, .. } => f
                .debug_struct("QuickWipeArm")
                .field("duress_passphrase", &REDACTED)
                .field("shortcut", shortcut)
                .finish(),
            Self::QuickWipeDisarm => f.write_str("QuickWipeDisarm"),
            Self::QuickWipeTrigger => f.write_str("QuickWipeTrigger"),
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
            Self::GroupDm(request) => std::fmt::Debug::fmt(request, f),
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
            | Self::SessionReinitiate { .. }
            | Self::SessionResetAccept { .. }
            | Self::IdentityRotationAccept { .. }
            | Self::IdentityRotationDecline { .. }
            | Self::SessionResetDecline { .. }
            | Self::IdentitySafetyNumber { .. }
            | Self::IdentityMarkVerified { .. }
            | Self::IdentityClearVerification { .. } => self.fmt_friend(f),
            Self::Community(request) => std::fmt::Debug::fmt(request, f),
            Self::Transfer(request) => std::fmt::Debug::fmt(request, f),
            Self::Servers(request) => std::fmt::Debug::fmt(request, f),
            Self::Lobby(request) => std::fmt::Debug::fmt(request, f),
            Self::DmTyping { .. }
            | Self::DmInbox { .. }
            | Self::DmHistory { .. }
            | Self::DmMarkRead { .. }
            | Self::OutboxRetry { .. }
            | Self::OutboxDiscard { .. } => self.fmt_dm(f),
            Self::Subscribe { filters, from } => f
                .debug_struct("Subscribe")
                .field("filter_count", &filters.len())
                .field("from", from)
                .finish(),
            Self::Unsubscribe { filters } => f
                .debug_struct("Unsubscribe")
                .field("filter_count", &filters.len())
                .finish(),

            // The remaining variants are formatted by `fmt_rest` to keep this
            // method under the clippy::too_many_lines limit. They are listed
            // explicitly rather than via `_` so adding an `IpcRequest` variant
            // remains a compile error here — exhaustiveness is preserved.
            Self::Undo { .. }
            | Self::PresenceSetMessage { .. }
            | Self::PresenceGet
            | Self::GameTotalsGet
            | Self::LocalSearch { .. }
            | Self::FriendGameTotalsGet { .. }
            | Self::ProfileBioGet
            | Self::FriendBioGet { .. }
            | Self::ProfileSetBio { .. }
            | Self::ProfileSetNickname { .. }
            | Self::SetReadReceiptsEnabled { .. }
            | Self::SetShareGameTotals { .. }
            | Self::SetGameDetection { .. }
            | Self::SetAutoAway { .. }
            | Self::SetShareCurrentGame { .. }
            | Self::SetKeepActivityLog { .. }
            | Self::ActivityList { .. }
            | Self::InfoviewGet { .. }
            | Self::HiddenGamesGet
            | Self::GameHide { .. }
            | Self::GameUnhide { .. }
            | Self::InstalledGamesGet
            | Self::GameLaunch { .. }
            | Self::GameInviteSend { .. }
            | Self::ProfileSetAvatar { .. }
            | Self::AvatarGet { .. }
            | Self::AuditVerify
            | Self::AuditExport { .. }
            | Self::SkinList
            | Self::SkinImport { .. }
            | Self::SkinRemove { .. }
            | Self::SkinSelect { .. }
            | Self::SkinSetLogin { .. }
            | Self::SkinKeep { .. }
            | Self::SkinGet { .. }
            | Self::SkinFile { .. }
            | Self::SkinFrames
            | Self::SkinValidateDraft { .. }
            | Self::SkinDraftStart { .. }
            | Self::SkinExportDraft { .. }
            | Self::SkinDraftCheck { .. }
            | Self::SkinExportSchema { .. }
            | Self::RunSkinAction { .. }
            | Self::SkinSelectionSet { .. }
            | Self::SkinApplyFailed { .. }
            | Self::LinkOpen { .. } => self.fmt_own(f),
            Self::PresenceSet { .. }
            | Self::GamePresenceSet { .. }
            | Self::GamePresenceClear
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
            Self::GameTotalsGet => write!(f, "GameTotalsGet"),
            // The query is the user's text: only its length is logged.
            Self::LocalSearch { scope, query } => f
                .debug_struct("LocalSearch")
                .field("scope", scope)
                .field("query_len", &query.len())
                .finish(),
            Self::FriendGameTotalsGet { public_key } => f
                .debug_struct("FriendGameTotalsGet")
                .field("public_key", public_key)
                .finish(),
            Self::Undo { id } => f.debug_struct("Undo").field("id", id).finish(),
            Self::ProfileBioGet => write!(f, "ProfileBioGet"),
            Self::FriendBioGet { public_key } => f
                .debug_struct("FriendBioGet")
                .field("public_key", public_key)
                .finish(),
            Self::ProfileSetBio { bio } => f
                .debug_struct("ProfileSetBio")
                .field("bio_len", &bio.chars().count())
                .finish(),
            Self::ProfileSetNickname { display_name } => f
                .debug_struct("ProfileSetNickname")
                .field("display_name_len", &display_name.len())
                .finish(),
            Self::SetReadReceiptsEnabled { enabled } => f
                .debug_struct("SetReadReceiptsEnabled")
                .field("enabled", enabled)
                .finish(),
            Self::SetShareGameTotals { enabled } => f
                .debug_struct("SetShareGameTotals")
                .field("enabled", enabled)
                .finish(),
            Self::SetGameDetection { enabled } => f
                .debug_struct("SetGameDetection")
                .field("enabled", enabled)
                .finish(),
            Self::SetAutoAway { minutes } => f
                .debug_struct("SetAutoAway")
                .field("minutes", minutes)
                .finish(),
            Self::SetKeepActivityLog { enabled } => f
                .debug_struct("SetKeepActivityLog")
                .field("enabled", enabled)
                .finish(),
            Self::ActivityList { before, limit } => f
                .debug_struct("ActivityList")
                .field("before", before)
                .field("limit", limit)
                .finish(),
            Self::InfoviewGet { subject, window } => f
                .debug_struct("InfoviewGet")
                .field("subject", subject)
                .field("window", window)
                .finish(),
            Self::SetShareCurrentGame { enabled } => f
                .debug_struct("SetShareCurrentGame")
                .field("enabled", enabled)
                .finish(),
            Self::HiddenGamesGet => write!(f, "HiddenGamesGet"),
            Self::GameHide { game_id } => f
                .debug_struct("GameHide")
                .field("game_id", game_id)
                .finish(),
            Self::GameUnhide { game_id } => f
                .debug_struct("GameUnhide")
                .field("game_id", game_id)
                .finish(),
            Self::InstalledGamesGet => write!(f, "InstalledGamesGet"),
            Self::GameLaunch {
                game_id, source, ..
            } => f
                .debug_struct("GameLaunch")
                .field("game_id", game_id)
                .field("source", source)
                .finish_non_exhaustive(),
            Self::GameInviteSend {
                public_key,
                game_id,
                ..
            } => f
                .debug_struct("GameInviteSend")
                .field("public_key", public_key)
                .field("game_id", game_id)
                .finish_non_exhaustive(),
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
            Self::AuditExport { since, path } => f
                .debug_struct("AuditExport")
                .field("since", since)
                .field("path", path)
                .finish(),
            Self::SkinList => write!(f, "SkinList"),
            Self::SkinImport { bytes } => f
                .debug_struct("SkinImport")
                .field("bytes_len", &bytes.len())
                .finish(),
            Self::SkinRemove { id } => f.debug_struct("SkinRemove").field("id", id).finish(),
            Self::SkinSelect { id } => f.debug_struct("SkinSelect").field("id", id).finish(),
            Self::SkinSetLogin { id } => f.debug_struct("SkinSetLogin").field("id", id).finish(),
            Self::SkinKeep { id, keep } => f
                .debug_struct("SkinKeep")
                .field("id", id)
                .field("keep", keep)
                .finish(),
            Self::SkinGet { id } => f.debug_struct("SkinGet").field("id", id).finish(),
            Self::SkinFile { path } => f.debug_struct("SkinFile").field("path", path).finish(),
            Self::SkinValidateDraft { draft } => f
                .debug_struct("SkinValidateDraft")
                .field("tokens", &draft.tokens.len())
                .finish(),
            Self::SkinDraftStart { id } => {
                f.debug_struct("SkinDraftStart").field("id", id).finish()
            }
            Self::SkinExportDraft { draft, path } => f
                .debug_struct("SkinExportDraft")
                .field("tokens", &draft.tokens.len())
                .field("path_len", &path.len())
                .finish(),
            Self::SkinDraftCheck { draft } => f
                .debug_struct("SkinDraftCheck")
                .field("tokens", &draft.tokens.len())
                .field("regions", &draft.regions.len())
                .finish(),
            Self::RunSkinAction {
                window,
                node,
                action,
            } => f
                .debug_struct("RunSkinAction")
                .field("window", window)
                .field("node_len", &node.len())
                .field("action", action)
                .finish(),
            Self::SkinSelectionSet { window, selection } => f
                .debug_struct("SkinSelectionSet")
                .field("window", window)
                .field("selected", &selection.is_some())
                .finish(),
            Self::SkinApplyFailed {
                window,
                skin,
                cause,
            } => f
                .debug_struct("SkinApplyFailed")
                .field("window", window)
                .field("skin", skin)
                .field("cause", cause)
                .finish(),
            Self::SkinExportSchema { path } => f
                .debug_struct("SkinExportSchema")
                .field("path_len", &path.len())
                .finish(),
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

    /// Compiles only for a passphrase that is wiped when it drops.
    fn wiped_on_drop<T: zeroize::ZeroizeOnDrop>(_: &T) {}

    /// Roadmap 1.7: every request that carries a passphrase holds it in a
    /// type that zeroizes on drop, so the Host's unlock path leaves no copy
    /// of it in freed memory once the request is handled.
    #[test]
    fn every_passphrase_field_is_wiped_on_drop() {
        let check: fn(&IpcRequest) = |request| match request {
            IpcRequest::Unlock { passphrase, .. }
            | IpcRequest::IdentityCreate { passphrase, .. }
            | IpcRequest::IdentityExport { passphrase }
            | IpcRequest::IdentityImport { passphrase, .. }
            | IpcRequest::IdentityRotate { passphrase } => wiped_on_drop(passphrase),
            _ => {}
        };
        check(&IpcRequest::IdentityRotate {
            passphrase: "p".into(),
        });
    }
}
