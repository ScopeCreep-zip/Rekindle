//! Community membership lifecycle events.
//!
//! Covers every state transition a member can undergo: join request,
//! approval, rejection, join announcement, leave, kick, ban, unban,
//! timeout, role change, and onboarding completion.

use serde::{Deserialize, Serialize};

/// Membership lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MembershipEvent {
    /// A join request was submitted (operator sees this).
    /// Triggered by: gossip `ControlPayload::MemberJoinRequest`.
    JoinRequested {
        community: String,
        pseudonym: String,
        display_name: String,
        has_invite: bool,
    },
    /// Our own join request was accepted (we see this).
    /// Triggered by: gossip `ControlPayload::JoinAccepted`.
    JoinAccepted {
        community: String,
        mek_generation: u64,
        slot_index: Option<u32>,
    },
    /// Our own join request was rejected (we see this).
    /// Triggered by: gossip `ControlPayload::JoinRejected`.
    JoinRejected { community: String, reason: String },
    /// A new member joined the community (everyone sees this).
    /// Triggered by: gossip `ControlPayload::MemberJoined`.
    Joined {
        community: String,
        pseudonym: String,
        display_name: String,
        role_ids: Vec<u32>,
    },
    /// A member left the community voluntarily.
    /// Triggered by: gossip `ControlPayload::MemberLeave`.
    Left {
        community: String,
        pseudonym: String,
    },
    /// A member was removed by an operator.
    /// Triggered by: gossip `ControlPayload::MemberRemoved`.
    Removed {
        community: String,
        pseudonym: String,
    },
    /// A member was kicked (immediate removal, can rejoin).
    /// Triggered by: gossip `ControlPayload::Kick`.
    Kicked {
        community: String,
        target_pseudonym: String,
    },
    /// A member was banned (permanent removal, cannot rejoin).
    /// Triggered by: gossip `ControlPayload::Ban`.
    Banned {
        community: String,
        target_pseudonym: String,
    },
    /// A member was unbanned.
    /// Triggered by: gossip `ControlPayload::Unban`.
    Unbanned {
        community: String,
        target_pseudonym: String,
    },
    /// A member was timed out (temporary restriction).
    /// Triggered by: gossip `ControlPayload::TimeoutMember`.
    TimedOut {
        community: String,
        target_pseudonym: String,
        duration_seconds: u64,
        reason: Option<String>,
    },
    /// A member's timeout was removed early.
    /// Triggered by: gossip `ControlPayload::RemoveTimeout`.
    TimeoutRemoved {
        community: String,
        target_pseudonym: String,
    },
    /// A member's timeout status changed (broadcast to community).
    /// Triggered by: gossip `ControlPayload::MemberTimedOut`.
    TimeoutStatusChanged {
        community: String,
        pseudonym: String,
        timeout_until: Option<u64>,
    },
    /// A member's roles changed.
    /// Triggered by: gossip `ControlPayload::MemberRolesChanged`.
    RolesChanged {
        community: String,
        pseudonym: String,
        role_ids: Vec<u32>,
    },
    /// A member completed onboarding.
    /// Triggered by: gossip `ControlPayload::OnboardingComplete`.
    OnboardingCompleted {
        community: String,
        pseudonym: String,
        role_ids: Vec<u32>,
    },
    /// Onboarding answers were submitted.
    /// Triggered by: gossip `ControlPayload::SubmitOnboardingAnswers`.
    OnboardingAnswersSubmitted {
        community: String,
        sender_pseudonym: String,
        answer_count: usize,
    },
}
