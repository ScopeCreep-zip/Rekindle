//! `canonical_state` covers every field of the merged state (plan E3.2
//! S7c.3 `the_state_root_covers_every_governance_state_field`).

use std::collections::HashSet;

use rekindle_types::event::{EventStatus, RsvpStatus};
use rekindle_types::governance::{AdmissionMode, AutoModAction, ModReportTarget};
use rekindle_types::id::{CategoryId, ChannelId, EventId, PseudonymKey, RoleId, ThreadId};

use super::canonical_state;
use crate::op::OpStamp;
use crate::state::{
    AdmissionDecision, AutoModRuleState, CategoryState, ChannelSegmentRecord, ChannelState,
    CommunityPolicyState, EventState, ExpressionState, GovernanceState, InviteState, MemberState,
    MetadataState, ModReportState, NotificationDefaultState, OnboardingState, OverwriteState,
    PendingMemberState, RoleState, RsvpState, SegmentState, ThreadState, TimeoutState,
    WelcomeScreenState,
};

fn pk() -> PseudonymKey {
    PseudonymKey([9; 32])
}

fn s() -> OpStamp {
    OpStamp::MIN
}

fn channel() -> ChannelState {
    ChannelState {
        name: "c".into(),
        channel_type: "text".into(),
        record_key: "VLD0:c".into(),
        category_id: None,
        position: 0,
        topic: None,
        forum_tags: None,
        slowmode_seconds: None,
        nsfw: None,
        parent_voice_channel_id: None,
        created: s(),
    }
}

fn role() -> RoleState {
    RoleState {
        name: "r".into(),
        permissions: 0,
        position: 0,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
        stamp: s(),
    }
}

fn thread() -> ThreadState {
    ThreadState {
        parent_channel_id: ChannelId([1; 16]),
        name: "t".into(),
        thread_type: "public".into(),
        record_key: None,
        invited: Vec::new(),
        forum_tag: None,
        auto_archive_seconds: 0,
        creator: pk(),
        created: s(),
        archived: None,
        unarchived: None,
    }
}

fn event() -> EventState {
    EventState {
        name: "e".into(),
        description: None,
        start_time: 0,
        end_time: None,
        channel_id: None,
        cover_image_ref: None,
        creator_pseudonym: None,
        recurrence: None,
        location: None,
        status: EventStatus::Scheduled,
        stamp: s(),
    }
}

fn expression() -> ExpressionState {
    ExpressionState {
        name: "x".into(),
        kind: "emoji".into(),
        content_hash: "h".into(),
        attachment: None,
        animated: false,
        tags: Vec::new(),
        sound_meta: None,
        creator_pseudonym: None,
        created_at: None,
        available_to_peers: false,
        stamp: s(),
    }
}

/// A change to one field.
type Mutation = Box<dyn Fn(&mut GovernanceState)>;

/// One mutation per field of `GovernanceState`, in declaration order.
fn mutations() -> Vec<(&'static str, Mutation)> {
    let id = [3u8; 16];
    vec![
        (
            "channels",
            Box::new(|g| _ = g.channels.insert(ChannelId([1; 16]), channel())),
        ),
        (
            "roles",
            Box::new(|g| _ = g.roles.insert(RoleId([1; 16]), role())),
        ),
        (
            "role_assignments",
            Box::new(|g| _ = g.role_assignments.insert(pk(), HashSet::new())),
        ),
        ("bans", Box::new(|g| _ = g.bans.insert(pk()))),
        (
            "members",
            Box::new(|g| {
                let m = MemberState {
                    member_record: "m".into(),
                    member_record_owner: "o".into(),
                    joined: s(),
                };
                g.members.insert(pk(), m);
            }),
        ),
        (
            "op_log_records",
            Box::new(|g| _ = g.op_log_records.insert(pk(), "m".into())),
        ),
        (
            "departed_heads",
            Box::new(|g| {
                _ = g
                    .departed_heads
                    .insert(pk(), std::collections::HashMap::default());
            }),
        ),
        ("removed", Box::new(|g| _ = g.removed.insert(pk(), s()))),
        (
            "removed_by",
            Box::new(|g| _ = g.removed_by.insert(pk(), pk())),
        ),
        (
            "removal_reasons",
            Box::new(|g| _ = g.removal_reasons.insert(pk(), "r".into())),
        ),
        (
            "pending_members",
            Box::new(|g| {
                let p = PendingMemberState {
                    member_record: "m".into(),
                    member_record_owner: "o".into(),
                    stamp: s(),
                };
                g.pending_members.insert(pk(), p);
            }),
        ),
        (
            "admitted",
            Box::new(|g| {
                _ = g.admitted.insert(
                    pk(),
                    AdmissionDecision {
                        approved: true,
                        stamp: s(),
                    },
                );
            }),
        ),
        (
            "admission_mode",
            Box::new(|g| g.admission_mode = Some(AdmissionMode::Open)),
        ),
        (
            "timeouts",
            Box::new(|g| {
                _ = g.timeouts.insert(
                    pk(),
                    TimeoutState {
                        duration_seconds: 1,
                        started_at: 0,
                        stamp: s(),
                    },
                );
            }),
        ),
        ("mek_generation", Box::new(|g| g.mek_generation = 2)),
        (
            "metadata",
            Box::new(|g| {
                g.metadata = Some(MetadataState {
                    name: "n".into(),
                    description: None,
                    icon_hash: None,
                    banner_hash: None,
                    stamp: s(),
                });
            }),
        ),
        (
            "notification_default",
            Box::new(|g| {
                g.notification_default = Some(NotificationDefaultState {
                    level: "all".into(),
                    stamp: s(),
                });
            }),
        ),
        (
            "categories",
            Box::new(|g| {
                _ = g.categories.insert(
                    CategoryId([1; 16]),
                    CategoryState {
                        name: "c".into(),
                        position: 0,
                        created: s(),
                    },
                );
            }),
        ),
        (
            "overwrites",
            Box::new(|g| {
                let o = OverwriteState {
                    target_type: "role".into(),
                    allow: 0,
                    deny: 0,
                    stamp: s(),
                };
                g.overwrites.insert((ChannelId([1; 16]), "t".into()), o);
            }),
        ),
        (
            "threads",
            Box::new(|g| _ = g.threads.insert(ThreadId([1; 16]), thread())),
        ),
        (
            "events",
            Box::new(|g| _ = g.events.insert(EventId([1; 16]), event())),
        ),
        (
            "event_rsvps",
            Box::new(|g| {
                let r = RsvpState {
                    status: RsvpStatus::Going,
                    stamp: s(),
                };
                g.event_rsvps
                    .entry(EventId([1; 16]))
                    .or_default()
                    .insert(pk(), r);
            }),
        ),
        (
            "expressions",
            Box::new(move |g| _ = g.expressions.insert(id, expression())),
        ),
        (
            "admin_deletes",
            Box::new(move |g| _ = g.admin_deletes.insert(id)),
        ),
        (
            "pinned_attachments",
            Box::new(move |g| _ = g.pinned_attachments.insert(id)),
        ),
        (
            "attachment_pin_stamps",
            Box::new(move |g| _ = g.attachment_pin_stamps.insert(id, s())),
        ),
        ("creator", Box::new(|g| g.creator = Some(pk()))),
        (
            "automod_rules",
            Box::new(move |g| {
                let r = AutoModRuleState {
                    name: "a".into(),
                    enabled: true,
                    trigger_json: "{}".into(),
                    action: AutoModAction::BlockLocally,
                    stamp: s(),
                };
                g.automod_rules.insert(id, r);
            }),
        ),
        (
            "onboarding",
            Box::new(|g| {
                g.onboarding = Some(OnboardingState {
                    enabled: true,
                    mode: "m".into(),
                    default_channels: Vec::new(),
                    questions: Vec::new(),
                    welcome_message: None,
                    guide_steps: Vec::new(),
                    stamp: s(),
                });
            }),
        ),
        (
            "welcome_screen",
            Box::new(|g| {
                g.welcome_screen = Some(WelcomeScreenState {
                    description: "w".into(),
                    channels: Vec::new(),
                    stamp: s(),
                });
            }),
        ),
        (
            "ban_stamps",
            Box::new(|g| _ = g.ban_stamps.insert(pk(), s())),
        ),
        (
            "segments",
            Box::new(|g| {
                g.segments.push(SegmentState {
                    segment_index: 1,
                    registry_key: "r".into(),
                    governance_key: "g".into(),
                    slot_range_start: 0,
                    slot_range_end: 1,
                });
            }),
        ),
        (
            "channel_segment_records",
            Box::new(|g| {
                let r = ChannelSegmentRecord {
                    record_key: "k".into(),
                    linked: s(),
                };
                g.channel_segment_records.insert((ChannelId([1; 16]), 1), r);
            }),
        ),
        (
            "invites",
            Box::new(move |g| {
                let i = InviteState {
                    code_hash: "c".into(),
                    max_uses: 1,
                    expires_at: None,
                    secrets_record_key: "k".into(),
                    created: s(),
                    creator_pseudonym: pk(),
                    uses: 0,
                };
                g.invites.insert(id, i);
            }),
        ),
        (
            "expression_remove_stamps",
            Box::new(move |g| _ = g.expression_remove_stamps.insert(id, s())),
        ),
        (
            "community_policy",
            Box::new(|g| {
                g.community_policy = Some(CommunityPolicyState {
                    policy_text: None,
                    max_joins_per_interval: 1,
                    join_interval_seconds: 1,
                    stamp: s(),
                });
            }),
        ),
        (
            "mod_reports",
            Box::new(move |g| {
                let r = ModReportState {
                    reporter: pk(),
                    target: ModReportTarget::Member { pseudonym: pk() },
                    reason: "r".into(),
                    stamp: s(),
                };
                g.mod_reports.insert(id, r);
            }),
        ),
    ]
}

/// Every field of the state changes the encoding, and none collides with
/// another: no field is left out of `stateRoot` or the roster checksum.
#[test]
fn the_state_root_covers_every_governance_state_field() {
    let base = canonical_state(&GovernanceState::default());
    let mut seen = HashSet::from([base]);
    let all = mutations();
    assert_eq!(all.len(), 37, "one mutation per GovernanceState field");
    for (field, mutate) in all {
        let mut state = GovernanceState::default();
        mutate(&mut state);
        assert!(
            seen.insert(canonical_state(&state)),
            "{field} is not encoded"
        );
    }
}

/// A use counted on an invite changes the encoding.
#[test]
fn an_invite_use_changes_the_state_encoding() {
    let mut state = GovernanceState::default();
    (mutations().swap_remove(33).1)(&mut state);
    let before = canonical_state(&state);
    if let Some(invite) = state.invites.values_mut().next() {
        invite.uses = 1;
    }
    assert_ne!(before, canonical_state(&state));
}
