//! Request classification: each `IpcRequest`'s stable name (for the audit
//! log), the dispatch lane it runs in, and the clearance it requires.

use super::IpcRequest;
use crate::message::SecurityLevel;

/// How a request may overlap the requests in flight with it.
///
/// The daemon dispatches every request as its own task. `Write` requests
/// share a lane with each other; an `Exclusive` request — one that loads,
/// replaces or removes the identity or its signing key — waits for every
/// in-flight write and holds the lane alone, so no write that cloned the
/// signing key can complete after a lock. `Query` requests take no lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Query,
    Write,
    Exclusive,
}

impl IpcRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    /// The dispatch lane this request runs in.
    #[must_use]
    pub fn lane(&self) -> Lane {
        self.class().1
    }

    /// The clearance the sending connection needs (plan D1), checked by the
    /// daemon against the level the bus server stamped. `Open`: the
    /// locked-state requests, each proved by a passphrase or a typed phrase
    /// (and `Status`). `Admin`: agents, policy, and moving the identity's
    /// key (rotate, export). `Authenticated`: everything else.
    #[must_use]
    pub fn required_clearance(&self) -> SecurityLevel {
        self.class().2
    }

    fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Exclusive, Query, Write};
        use SecurityLevel::{Admin, Authenticated, Open};
        match self {
            Self::Unlock { .. } => ("Unlock", Exclusive, Open),
            Self::Lock => ("Lock", Exclusive, Open),
            Self::Logout => ("Logout", Exclusive, Open),
            Self::Status => ("Status", Query, Open),
            Self::Shutdown => ("Shutdown", Exclusive, Authenticated),

            Self::IdentityCreate { .. } => ("IdentityCreate", Exclusive, Open),
            Self::IdentityList => ("IdentityList", Query, Open),
            Self::IdentityShow => ("IdentityShow", Query, Authenticated),
            Self::IdentityExport { .. } => ("IdentityExport", Query, Admin),
            Self::IdentityImport { .. } => ("IdentityImport", Exclusive, Open),
            Self::IdentityRotate { .. } => ("IdentityRotate", Exclusive, Admin),
            Self::IdentityDelete { .. } => ("IdentityDelete", Exclusive, Open),
            Self::IdentityWipe => ("IdentityWipe", Exclusive, Open),
            Self::QuickWipeArm { .. } => ("QuickWipeArm", Write, Admin),
            Self::QuickWipeDisarm => ("QuickWipeDisarm", Write, Admin),
            Self::QuickWipeTrigger => ("QuickWipeTrigger", Exclusive, Authenticated),
            Self::IdentitySafetyNumber { .. } => ("IdentitySafetyNumber", Query, Authenticated),
            Self::IdentityMarkVerified { .. } => ("IdentityMarkVerified", Write, Authenticated),
            Self::IdentityClearVerification { .. } => {
                ("IdentityClearVerification", Write, Authenticated)
            }
            // The gate checks who may commit (plan D3.3); the held request
            // runs in its own lane.
            Self::IntentCommit { .. } => ("IntentCommit", Write, Open),
            Self::IntentListHeld => ("IntentListHeld", Query, Authenticated),
            Self::IntentCancel { .. } => ("IntentCancel", Write, Open),

            Self::FriendAddFromCode { .. } => ("FriendAddFromCode", Write, Authenticated),
            Self::FriendAccept { .. } => ("FriendAccept", Write, Authenticated),
            Self::FriendReject { .. } => ("FriendReject", Write, Authenticated),
            Self::FriendRemove { .. } => ("FriendRemove", Write, Authenticated),
            Self::FriendList => ("FriendList", Query, Authenticated),
            Self::FriendRequests => ("FriendRequests", Query, Authenticated),
            Self::FriendCancelRequest { .. } => ("FriendCancelRequest", Write, Authenticated),
            Self::FriendRetryHandshake { .. } => ("FriendRetryHandshake", Write, Authenticated),
            Self::FriendAddFromInvite { .. } => ("FriendAddFromInvite", Write, Authenticated),
            Self::FriendInviteCreate => ("FriendInviteCreate", Write, Authenticated),
            Self::FriendCode => ("FriendCode", Query, Authenticated),
            Self::FriendTokenRotate => ("FriendTokenRotate", Write, Authenticated),
            Self::FriendInviteList => ("FriendInviteList", Query, Authenticated),
            Self::FriendInviteCancel { .. } => ("FriendInviteCancel", Write, Authenticated),
            Self::FriendBlock { .. } => ("FriendBlock", Write, Authenticated),
            Self::FriendUnblock { .. } => ("FriendUnblock", Write, Authenticated),
            Self::FriendBlockedList => ("FriendBlockedList", Query, Authenticated),
            Self::FriendSetNickname { .. } => ("FriendSetNickname", Write, Authenticated),
            Self::FriendGroupCreate { .. } => ("FriendGroupCreate", Write, Authenticated),
            Self::FriendGroupRename { .. } => ("FriendGroupRename", Write, Authenticated),
            Self::FriendGroupMove { .. } => ("FriendGroupMove", Write, Authenticated),
            Self::FriendGroupDelete { .. } => ("FriendGroupDelete", Write, Authenticated),
            Self::SessionResetStart { .. } => ("SessionResetStart", Write, Authenticated),
            Self::SessionReinitiate { .. } => ("SessionReinitiate", Write, Authenticated),
            Self::SessionResetAccept { .. } => ("SessionResetAccept", Write, Authenticated),
            Self::SessionResetDecline { .. } => ("SessionResetDecline", Write, Authenticated),
            Self::IdentityRotationAccept { .. } => ("IdentityRotationAccept", Write, Authenticated),
            Self::IdentityRotationDecline { .. } => {
                ("IdentityRotationDecline", Write, Authenticated)
            }

            Self::Community(request) => request.class(),
            Self::PermissionCatalogue => ("PermissionCatalogue", Query, Open),
            Self::Transfer(request) => request.class(),
            Self::LocalSearch { .. } => ("LocalSearch", Query, Authenticated),
            Self::Servers(request) => request.class(),
            Self::Lobby(request) => request.class(),

            Self::DmSend { .. } => ("DmSend", Write, Authenticated),
            Self::DmTyping { .. } => ("DmTyping", Write, Authenticated),
            Self::DmInbox { .. } => ("DmInbox", Query, Authenticated),
            Self::DmHistory { .. } => ("DmHistory", Query, Authenticated),
            Self::DmMarkRead { .. } => ("DmMarkRead", Write, Authenticated),
            Self::OutboxRetry { .. } => ("OutboxRetry", Write, Authenticated),
            Self::OutboxDiscard { .. } => ("OutboxDiscard", Write, Authenticated),
            Self::GroupDm(request) => request.class(),
            Self::DisappearingSet { .. } => ("DisappearingSet", Write, Authenticated),
            Self::DisappearingGet { .. } => ("DisappearingGet", Query, Authenticated),

            Self::Undo { .. } => ("Undo", Write, Authenticated),
            Self::Subscribe { .. } => ("Subscribe", Query, Authenticated),
            Self::Unsubscribe { .. } => ("Unsubscribe", Query, Authenticated),

            Self::PrekeyStatus => ("PrekeyStatus", Query, Authenticated),

            Self::PresenceSet { .. } => ("PresenceSet", Write, Authenticated),
            Self::PresenceSetMessage { .. } => ("PresenceSetMessage", Write, Authenticated),
            Self::PresenceGet => ("PresenceGet", Query, Authenticated),
            Self::GameTotalsGet => ("GameTotalsGet", Query, Authenticated),
            Self::FriendGameTotalsGet { .. } => ("FriendGameTotalsGet", Query, Authenticated),
            Self::ProfileBioGet => ("ProfileBioGet", Query, Authenticated),
            Self::FriendBioGet { .. } => ("FriendBioGet", Query, Authenticated),
            Self::ProfileSetBio { .. } => ("ProfileSetBio", Write, Authenticated),
            Self::ProfileSetNickname { .. } => ("ProfileSetNickname", Write, Authenticated),
            Self::SetReadReceiptsEnabled { .. } => ("SetReadReceiptsEnabled", Write, Authenticated),
            Self::SetShareGameTotals { .. } => ("SetShareGameTotals", Write, Authenticated),
            Self::SetGameDetection { .. } => ("SetGameDetection", Write, Authenticated),
            Self::SetAutoAway { .. } => ("SetAutoAway", Write, Authenticated),
            Self::SetShareCurrentGame { .. } => ("SetShareCurrentGame", Write, Authenticated),
            Self::SetKeepActivityLog { .. } => ("SetKeepActivityLog", Write, Authenticated),
            Self::ActivityList { .. } => ("ActivityList", Query, Authenticated),
            Self::InfoviewGet { .. } => ("InfoviewGet", Query, Authenticated),
            Self::HiddenGamesGet => ("HiddenGamesGet", Query, Authenticated),
            Self::GameHide { .. } => ("GameHide", Write, Authenticated),
            Self::GameUnhide { .. } => ("GameUnhide", Write, Authenticated),
            Self::InstalledGamesGet => ("InstalledGamesGet", Query, Authenticated),
            Self::GameLaunch { .. } => ("GameLaunch", Write, Authenticated),
            Self::GameInviteSend { .. } => ("GameInviteSend", Write, Authenticated),
            Self::ProfileSetAvatar { .. } => ("ProfileSetAvatar", Write, Authenticated),
            Self::AvatarGet { .. } => ("AvatarGet", Query, Authenticated),
            Self::SkinList => ("SkinList", Query, Authenticated),
            Self::SkinImport { .. } => ("SkinImport", Write, Authenticated),
            Self::SkinRemove { .. } => ("SkinRemove", Write, Authenticated),
            Self::SkinSelect { .. } => ("SkinSelect", Write, Authenticated),
            Self::SkinSetLogin { .. } => ("SkinSetLogin", Write, Authenticated),
            Self::SkinKeep { .. } => ("SkinKeep", Write, Authenticated),
            // The login window applies the device login skin before unlock
            // (plaintext by design, threat model Z16).
            Self::SkinGet { .. } => ("SkinGet", Query, Open),
            Self::SkinFile { .. } => ("SkinFile", Query, Open),
            Self::SkinFrames => ("SkinFrames", Query, Open),
            Self::SkinValidateDraft { .. } => ("SkinValidateDraft", Query, Authenticated),
            Self::SkinDraftStart { .. } => ("SkinDraftStart", Query, Authenticated),
            Self::SkinExportDraft { .. } => ("SkinExportDraft", Write, Authenticated),
            Self::SkinDraftCheck { .. } => ("SkinDraftCheck", Query, Authenticated),
            Self::SkinExportSchema { .. } => ("SkinExportSchema", Write, Authenticated),
            Self::RunSkinAction { .. } => ("RunSkinAction", Write, Authenticated),
            Self::SkinSelectionSet { .. } => ("SkinSelectionSet", Write, Authenticated),
            // The login window reports its device skin before unlock.
            Self::SkinApplyFailed { .. } => ("SkinApplyFailed", Write, Open),
            Self::GamePresenceSet { .. } => ("GamePresenceSet", Write, Authenticated),
            Self::GamePresenceClear => ("GamePresenceClear", Write, Authenticated),

            Self::AuditVerify => ("AuditVerify", Query, Authenticated),
            Self::AuditExport { .. } => ("AuditExport", Query, Authenticated),
            Self::LinkOpen { .. } => ("LinkOpen", Query, Authenticated),

            Self::VoiceJoin { .. } => ("VoiceJoin", Write, Authenticated),
            Self::VoiceLeave => ("VoiceLeave", Write, Authenticated),

            Self::NetworkStatus => ("NetworkStatus", Query, Authenticated),
            Self::NetworkPeers => ("NetworkPeers", Query, Authenticated),

            Self::AgentRegister { .. } => ("AgentRegister", Write, Admin),
            Self::AgentRevoke { .. } => ("AgentRevoke", Write, Admin),
            Self::PolicyReload => ("PolicyReload", Write, Admin),
        }
    }
}

#[cfg(test)]
#[path = "lane_tests.rs"]
mod tests;
