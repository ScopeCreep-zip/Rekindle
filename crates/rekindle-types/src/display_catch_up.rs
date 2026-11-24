//! The unlock barrier's view for the community window (plan E3.2
//! S7d-keys.8): while the barrier is catching up, our governance writes are
//! held in the outbox as "pending: catching up", never signed at a guessed
//! seq; the window lists each held write and whether it is still waiting
//! or being signed. Sent in full on every change
//! ([`GovernanceEvent::CatchUp`](crate::subscription_events::GovernanceEvent::CatchUp))
//! and carried on re-read ([`CommunityDetail::catch_up`](crate::display::CommunityDetail)),
//! so a window opened mid-barrier shows it.
//!
//! Postcard on the daemon IPC: unit enums and plain structs only.

use serde::{Deserialize, Serialize};

use crate::governance::GovernanceEntry;

/// The barrier and every governance write it holds, in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatchUpView {
    pub catching_up: bool,
    pub pending: Vec<PendingGovOp>,
}

/// One held governance write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingGovOp {
    /// The Host's id for the held write (stable while it is held).
    pub op_id: String,
    pub kind: PendingGovKind,
    pub target: PendingGovTarget,
    /// The Host's words for it, e.g. "Kick Bea".
    pub label: String,
    pub state: PendingGovState,
}

/// What a held write names: a member (hex pseudonym), a channel or a role.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingGovTarget {
    pub member: Option<String>,
    pub channel: Option<String>,
    pub role: Option<String>,
}

/// Where a held write stands: waiting for the barrier, or past it and
/// being signed (seq n + 1 is signed only after seq n landed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingGovState {
    CatchingUp,
    Signing,
}

/// Every governance write kind that can be held, one per
/// [`GovernanceEntry`] arm (two role-assignment arms share `RolesChanged`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingGovKind {
    ChannelCreated,
    ChannelEdited,
    ChannelDeleted,
    RoleEdited,
    RolesChanged,
    RoleDeleted,
    Ban,
    Unban,
    Join,
    MemberRecordMoved,
    Leave,
    Kick,
    ApproveMember,
    RejectMember,
    Timeout,
    RemoveTimeout,
    CommunityEdited,
    NotificationDefault,
    KeyRotation,
    CategoryCreated,
    CategoryEdited,
    CategoryDeleted,
    PermissionsChanged,
    ThreadCreated,
    ThreadArchived,
    ThreadUnarchived,
    EventCreated,
    EventArchived,
    EventRsvp,
    ExpressionAdded,
    ExpressionRemoved,
    ModReport,
    OnboardingConfig,
    WelcomeScreen,
    AdminDelete,
    SegmentAdded,
    AutoModRule,
    InviteCreated,
    InviteRevoked,
    AttachmentPinned,
    CommunityPolicy,
    Checkpoint,
    Successor,
    DeviceKey,
    DeviceRetired,
    DeviceReadmit,
    SuccessorLinkRequest,
    SuccessorLink,
}

impl PendingGovKind {
    /// The kind of a held `entry`.
    #[must_use]
    pub fn of(entry: &GovernanceEntry) -> Self {
        use GovernanceEntry as E;
        match entry {
            E::ChannelCreated { .. } => Self::ChannelCreated,
            E::ChannelUpdated { .. } => Self::ChannelEdited,
            E::ChannelArchived { .. } => Self::ChannelDeleted,
            E::RoleDefinition { .. } => Self::RoleEdited,
            E::RoleAssignment { .. } | E::RoleUnassignment { .. } => Self::RolesChanged,
            E::RoleArchived { .. } => Self::RoleDeleted,
            E::BanEntry { .. } => Self::Ban,
            E::UnbanEntry { .. } => Self::Unban,
            E::MemberJoined { .. } => Self::Join,
            E::MemberRecordMoved { .. } => Self::MemberRecordMoved,
            E::MemberLeft { .. } => Self::Leave,
            E::MemberRemoved { .. } => Self::Kick,
            E::MemberApproved { .. } => Self::ApproveMember,
            E::MemberRejected { .. } => Self::RejectMember,
            E::TimeoutEntry { .. } => Self::Timeout,
            E::RemoveTimeoutEntry { .. } => Self::RemoveTimeout,
            E::CommunityMeta { .. } => Self::CommunityEdited,
            E::CommunityNotificationDefault { .. } => Self::NotificationDefault,
            E::MEKGenerationBump { .. } => Self::KeyRotation,
            E::CategoryCreated { .. } => Self::CategoryCreated,
            E::CategoryUpdated { .. } => Self::CategoryEdited,
            E::CategoryArchived { .. } => Self::CategoryDeleted,
            E::PermissionOverwrite { .. } => Self::PermissionsChanged,
            E::ThreadCreated { .. } => Self::ThreadCreated,
            E::ThreadArchived { .. } => Self::ThreadArchived,
            E::ThreadUnarchived { .. } => Self::ThreadUnarchived,
            E::EventCreated { .. } => Self::EventCreated,
            E::EventArchived { .. } => Self::EventArchived,
            E::EventRsvp { .. } => Self::EventRsvp,
            E::ExpressionAdded { .. } => Self::ExpressionAdded,
            E::ExpressionRemoved { .. } => Self::ExpressionRemoved,
            E::ModReport { .. } => Self::ModReport,
            E::OnboardingConfig { .. } => Self::OnboardingConfig,
            E::WelcomeScreen { .. } => Self::WelcomeScreen,
            E::AdminDelete { .. } => Self::AdminDelete,
            E::SegmentAdded { .. } => Self::SegmentAdded,
            E::AutoModRule { .. } => Self::AutoModRule,
            E::InviteCreated { .. } => Self::InviteCreated,
            E::InviteRevoked { .. } => Self::InviteRevoked,
            E::AttachmentPinned { .. } => Self::AttachmentPinned,
            E::CommunityPolicy { .. } => Self::CommunityPolicy,
            E::Checkpoint { .. } => Self::Checkpoint,
            E::CommunitySuccessor { .. } => Self::Successor,
            E::DeviceKey { .. } => Self::DeviceKey,
            E::DeviceRetired { .. } => Self::DeviceRetired,
            E::DeviceReadmit { .. } => Self::DeviceReadmit,
            E::SuccessorLinkRequest { .. } => Self::SuccessorLinkRequest,
            E::SuccessorLink { .. } => Self::SuccessorLink,
        }
    }
}

impl PendingGovTarget {
    /// The member, channel or role a held `entry` names (hex).
    #[must_use]
    pub fn of(entry: &GovernanceEntry) -> Self {
        use GovernanceEntry as E;
        let member = |m: &crate::id::PseudonymKey| Self {
            member: Some(m.to_hex()),
            ..Self::default()
        };
        let channel = |c: &crate::id::ChannelId| Self {
            channel: Some(c.to_hex()),
            ..Self::default()
        };
        match entry {
            E::RoleAssignment {
                target, role_id, ..
            }
            | E::RoleUnassignment {
                target, role_id, ..
            } => Self {
                member: Some(target.to_hex()),
                role: Some(role_id.to_hex()),
                channel: None,
            },
            E::BanEntry { target, .. }
            | E::UnbanEntry { target }
            | E::MemberRemoved { target, .. }
            | E::MemberApproved { target }
            | E::MemberRejected { target, .. }
            | E::TimeoutEntry { target, .. }
            | E::RemoveTimeoutEntry { target } => member(target),
            E::ChannelCreated { channel_id, .. }
            | E::ChannelArchived { channel_id }
            | E::ChannelUpdated { channel_id, .. }
            | E::PermissionOverwrite { channel_id, .. }
            | E::AdminDelete { channel_id, .. } => channel(channel_id),
            E::RoleDefinition { role_id, .. } | E::RoleArchived { role_id } => Self {
                role: Some(role_id.to_hex()),
                ..Self::default()
            },
            _ => Self::default(),
        }
    }
}

impl PendingGovOp {
    /// `entry` held under `op_id`, waiting for the barrier; its label is
    /// the kind in words and the short id of what it names ("Kick
    /// a1b2c3d4").
    #[must_use]
    pub fn held(op_id: String, entry: &GovernanceEntry) -> Self {
        Self::held_as(
            op_id,
            PendingGovKind::of(entry),
            PendingGovTarget::of(entry),
        )
    }

    /// A held write of `kind` naming `target`.
    #[must_use]
    pub fn held_as(op_id: String, kind: PendingGovKind, target: PendingGovTarget) -> Self {
        let mut label = kind_words(kind);
        let named = [&target.member, &target.channel, &target.role];
        if let Some(id) = named.into_iter().flatten().next() {
            label.push(' ');
            label.push_str(id.get(..8).unwrap_or(id));
        }
        Self {
            op_id,
            kind,
            target,
            label,
            state: PendingGovState::CatchingUp,
        }
    }

    /// Relabel with what the target shows as (a member's card name, a
    /// channel's or a role's name): "<kind in words> <name>".
    #[must_use]
    pub fn with_name(mut self, name: &str) -> Self {
        self.label = format!("{} {name}", kind_words(self.kind));
        self
    }
}

/// `kind` in words: "Kick", "Role edited".
fn kind_words(kind: PendingGovKind) -> String {
    let mut words = String::new();
    for (i, c) in format!("{kind:?}").chars().enumerate() {
        if i > 0 && c.is_ascii_uppercase() {
            words.push(' ');
            words.push(c.to_ascii_lowercase());
        } else {
            words.push(c);
        }
    }
    words
}

#[cfg(test)]
#[path = "display_catch_up_tests.rs"]
mod tests;
