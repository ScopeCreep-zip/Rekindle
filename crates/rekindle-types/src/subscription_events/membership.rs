//! Community membership lifecycle events.
//!
//! Covers every state transition a member can undergo: join request,
//! approval, rejection, join announcement, leave, kick, ban, unban,
//! timeout, role change, and onboarding completion.

use serde::{Deserialize, Serialize};

/// Membership lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
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
        /// The MEK generation the joiner starts on.
        ///
        /// `Option` because the desktop's governance runtime signals
        /// acceptance from a path that has not read the MEK yet; the
        /// gossip path supplies it. `None` means "not yet known", not
        /// generation zero.
        mek_generation: Option<u64>,
        slot_index: Option<u32>,
    },
    /// Our own join request was rejected (we see this).
    /// Triggered by: gossip `ControlPayload::JoinRejected`.
    JoinRejected { community: String, reason: String },
    /// The full member list was re-read and should be re-rendered.
    ///
    /// Not a membership *change*: a refresh follows a registry scan or
    /// a governance rebuild, where the delta is unknown and the
    /// frontend re-queries rather than patching rows.
    MembersRefreshed { community: String },

    /// A member was found by scanning the registry, rather than
    /// announced by gossip.
    ///
    /// Distinct from [`Self::Joined`]: discovery says "this slot is
    /// occupied and always was", so a frontend adds the row without
    /// the join chrome (system message, sound, join toast).
    MemberDiscovered {
        community: String,
        pseudonym: String,
        display_name: String,
        /// Which registry slot they occupy.
        subkey_index: u32,
    },

    /// Per-phase progress through the self-sovereign join.
    ///
    /// Architecture §6.2: emitted before and after each gated phase
    /// (governance snapshot → invite decode → slot claim → presence →
    /// open records → watch) so a frontend renders a stepper instead of
    /// one long spinner. A CLI prints the same phases as lines.
    JoinProgress {
        community: String,
        /// The phase name.
        stage: String,
        /// `started` | `done` | `failed` | `timedOut`.
        status: String,
    },

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
    /// Triggered by: gossip `ControlPayload::Kick`, and by the Host when the
    /// merged governance names our own pseudonym in a `MemberRemoved`.
    Kicked {
        community: String,
        target_pseudonym: String,
        /// The remover's pseudonym, when the merged op names it: our own
        /// removal (plan E3.2 S9 own removal) carries it; gossip carries none.
        removed_by: Option<String>,
        /// The display name on the remover's verified card, when held.
        removed_by_name: Option<String>,
        /// The reason the removal op gave (A/B R5-2), shown to the removed
        /// member. Bounded at the source: governance validation refuses a
        /// removal whose reason exceeds `key_format::MAX_NOTE_LEN`, as for
        /// a mod report. Gossip carries none.
        reason: Option<String>,
    },
    /// A member was banned (permanent removal, cannot rejoin).
    /// Triggered by: gossip `ControlPayload::Ban`, and by the Host when the
    /// merged governance bans our own pseudonym.
    Banned {
        community: String,
        target_pseudonym: String,
        /// The remover's pseudonym, when the merged op names it: our own
        /// removal (plan E3.2 S9 own removal) carries it; gossip carries none.
        removed_by: Option<String>,
        /// The display name on the remover's verified card, when held.
        removed_by_name: Option<String>,
        /// The reason the removal op gave (A/B R5-2), shown to the removed
        /// member. Bounded at the source: governance validation refuses a
        /// removal whose reason exceeds `key_format::MAX_NOTE_LEN`, as for
        /// a mod report. Gossip carries none.
        reason: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::MembershipEvent;

    /// The removal's reason crosses the wire under `reason` and back.
    #[test]
    fn a_removal_reason_round_trips() {
        for banned in [false, true] {
            let event = if banned {
                MembershipEvent::Banned {
                    community: "c".into(),
                    target_pseudonym: "t".into(),
                    removed_by: Some("b".into()),
                    removed_by_name: Some("Ann".into()),
                    reason: Some("spam".into()),
                }
            } else {
                MembershipEvent::Kicked {
                    community: "c".into(),
                    target_pseudonym: "t".into(),
                    removed_by: None,
                    removed_by_name: None,
                    reason: Some("spam".into()),
                }
            };
            let json = serde_json::to_value(&event).expect("serializes");
            let tag = if banned { "banned" } else { "kicked" };
            assert_eq!(json[tag]["reason"], "spam", "{json}");
            let back: MembershipEvent = serde_json::from_value(json).expect("deserializes");
            let reason = match back {
                MembershipEvent::Kicked { reason, .. } | MembershipEvent::Banned { reason, .. } => {
                    reason
                }
                _ => None,
            };
            assert_eq!(reason.as_deref(), Some("spam"));
        }
    }
}
