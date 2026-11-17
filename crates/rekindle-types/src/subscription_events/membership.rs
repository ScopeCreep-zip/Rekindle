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
    /// Triggered by: the Host, when the merged governance first lists the
    /// member (`member_events.rs`); the gossip notice publishes none.
    Joined {
        community: String,
        pseudonym: String,
        /// The name on the member's verified card (`M`/0), when we have
        /// read it; `None` = not read yet, never "" (plan E3.2 F9-2). The
        /// event goes out at once, so the name usually follows as a
        /// [`Self::MemberCardChanged`]; a frontend shows the short
        /// pseudonym until then.
        display_name: Option<String>,
        role_ids: Vec<u32>,
    },
    /// The name on a member's verified card (`M`/0, signed by the
    /// pseudonym, its owner pinned to the member's `MemberJoined`), on the
    /// first read of that card and on each change (plan E3.2 F9-2).
    /// Triggered by: the Host, when it notes a card name it did not hold
    /// (`CommunityStore::note_member_card_name`).
    MemberCardChanged {
        community: String,
        pseudonym: String,
        display_name: String,
    },
    /// A member left the community voluntarily.
    /// Triggered by: the Host, when the merged governance first holds the
    /// member's `MemberLeft`, on every member's frontends, the leaver's
    /// own included; the gossip notice publishes none.
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
    /// Triggered by: the Host, once per removal, when the merged governance
    /// first holds the `MemberRemoved` (`member_events.rs`, our own removal
    /// `own_removal.rs`); the gossip notice publishes none.
    Kicked {
        community: String,
        target_pseudonym: String,
        /// The remover's pseudonym, when the merged op names it: our own
        /// removal (plan E3.2 S9 own removal) carries it, as does another's.
        removed_by: Option<String>,
        /// The display name on the remover's verified card, when held.
        removed_by_name: Option<String>,
        /// The reason the removal op gave (A/B R5-2), shown to the removed
        /// member. Bounded at the source: governance validation refuses a
        /// removal whose reason exceeds `key_format::MAX_NOTE_LEN`, as for
        /// a mod report.
        reason: Option<String>,
    },
    /// A member was banned (permanent removal, cannot rejoin).
    /// Triggered by: the Host, once per ban, when the merged governance
    /// first holds it (`member_events.rs`, our own ban `own_removal.rs`);
    /// the gossip notice publishes none.
    Banned {
        community: String,
        target_pseudonym: String,
        /// The remover's pseudonym, when the merged op names it: our own
        /// removal (plan E3.2 S9 own removal) carries it, as does another's.
        removed_by: Option<String>,
        /// The display name on the remover's verified card, when held.
        removed_by_name: Option<String>,
        /// The reason the removal op gave (A/B R5-2), shown to the removed
        /// member. Bounded at the source: governance validation refuses a
        /// removal whose reason exceeds `key_format::MAX_NOTE_LEN`, as for
        /// a mod report.
        reason: Option<String>,
    },
    /// A member was unbanned.
    /// Triggered by: the Host, when the merged governance lifts the ban
    /// (`member_events.rs`); the gossip notice publishes none.
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

    /// A Joined whose card is not read yet carries a null name, never "".
    #[test]
    fn a_joined_before_its_card_carries_no_name() {
        let event = MembershipEvent::Joined {
            community: "c".into(),
            pseudonym: "p".into(),
            display_name: None,
            role_ids: Vec::new(),
        };
        let json = serde_json::to_value(&event).expect("serializes");
        assert!(json["joined"]["displayName"].is_null(), "{json}");
    }

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
