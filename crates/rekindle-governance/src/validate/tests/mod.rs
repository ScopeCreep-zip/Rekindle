//! Tests for [`super::validate_write`].

use super::*;
use crate::state::{GovernanceState, RoleState};
use rekindle_types::permissions::{READ_HISTORY, VIEW_CHANNELS};
use std::collections::{HashMap, HashSet};

fn pseudo(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

fn rid(b: u8) -> rekindle_types::id::RoleId {
    rekindle_types::id::RoleId([b; 16])
}

fn state_with_creator_and_roles() -> GovernanceState {
    let mut roles = HashMap::new();
    roles.insert(
        rid(0),
        RoleState {
            name: "everyone".into(),
            permissions: VIEW_CHANNELS | SEND_MESSAGES | READ_HISTORY,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 1,
        },
    );
    roles.insert(
        rid(1),
        RoleState {
            name: "admin".into(),
            permissions: ADMINISTRATOR,
            position: 1,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 2,
        },
    );

    let mut assignments = HashMap::new();
    let mut admin_roles = HashSet::new();
    admin_roles.insert(rid(1));
    assignments.insert(pseudo(5), admin_roles); // pseudo(5) is admin

    GovernanceState {
        creator: Some(pseudo(1)),
        roles,
        role_assignments: assignments,
        ..Default::default()
    }
}

#[test]
fn creator_always_validates() {
    let state = state_with_creator_and_roles();
    let entry = GovernanceEntry::ChannelCreated {
        channel_id: rekindle_types::id::ChannelId([0; 16]),
        name: "test".into(),
        channel_type: "text".into(),
        record_key: "k".into(),
        category_id: None,
        position: 0,
        parent_voice_channel_id: None,
        lamport: 10,
    };
    assert!(validate_write(&pseudo(1), &entry, &state));
}

#[test]
fn regular_member_cannot_manage_channels() {
    let state = state_with_creator_and_roles();
    let entry = GovernanceEntry::ChannelCreated {
        channel_id: rekindle_types::id::ChannelId([0; 16]),
        name: "test".into(),
        channel_type: "text".into(),
        record_key: "k".into(),
        category_id: None,
        position: 0,
        parent_voice_channel_id: None,
        lamport: 10,
    };
    // pseudo(99) has only @everyone perms — no MANAGE_CHANNELS
    assert!(!validate_write(&pseudo(99), &entry, &state));
}

#[test]
fn admin_can_manage_channels() {
    let state = state_with_creator_and_roles();
    let entry = GovernanceEntry::ChannelCreated {
        channel_id: rekindle_types::id::ChannelId([0; 16]),
        name: "test".into(),
        channel_type: "text".into(),
        record_key: "k".into(),
        category_id: None,
        position: 0,
        parent_voice_channel_id: None,
        lamport: 10,
    };
    // pseudo(5) is admin
    assert!(validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn mek_bump_accepted_at_governance_layer() {
    let state = state_with_creator_and_roles();
    let entry = GovernanceEntry::MEKGenerationBump {
        generation: 2,
        trigger_departed: pseudo(10),
        cascade_skipped: vec![],
        lamport: 10,
    };
    // At governance CRDT layer, MEK bumps are accepted (Max-Register).
    // Full rotator authority is verified at the sync layer.
    assert!(validate_write(&pseudo(99), &entry, &state));
}

#[test]
fn banned_member_cannot_write() {
    let mut state = state_with_creator_and_roles();
    state.bans.insert(pseudo(50));

    let entry = GovernanceEntry::MEKGenerationBump {
        generation: 2,
        trigger_departed: pseudo(10),
        cascade_skipped: vec![],
        lamport: 10,
    };
    assert!(!validate_write(&pseudo(50), &entry, &state));
}

#[test]
fn regular_member_can_create_thread() {
    let state = state_with_creator_and_roles();
    let entry = GovernanceEntry::ThreadCreated {
        thread_id: rekindle_types::id::ThreadId([0; 16]),
        parent_channel_id: rekindle_types::id::ChannelId([0; 16]),
        name: "discussion".into(),
        thread_type: "public".into(),
        record_key: None,
        invited: Vec::new(),
        forum_tag: None,
        auto_archive_seconds: 86_400,
        lamport: 10,
    };
    // @everyone has SEND_MESSAGES
    assert!(validate_write(&pseudo(99), &entry, &state));
}

#[test]
fn self_assignable_role_allows_self_write() {
    let mut state = state_with_creator_and_roles();
    let self_role_id = rid(9);
    state.roles.insert(
        self_role_id,
        RoleState {
            name: "self".into(),
            permissions: 0,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: true,
            exclusion_group: None,
            lamport: 3,
        },
    );

    let entry = GovernanceEntry::RoleAssignment {
        target: pseudo(99),
        role_id: self_role_id,
        lamport: 10,
    };
    assert!(validate_write(&pseudo(99), &entry, &state));
}

#[test]
fn expression_limit_rejects_extra_static_emoji() {
    let mut state = state_with_creator_and_roles();
    for index in 0..50_u8 {
        state.expressions.insert(
            [index; 16],
            crate::state::ExpressionState {
                name: format!("emoji-{index}"),
                kind: "emoji".into(),
                content_hash: format!("hash-{index}"),
                attachment: None,
                animated: false,
                tags: vec![],
                sound_meta: None,
                creator_pseudonym: None,
                created_at: None,
                available_to_peers: true,
                lamport: u64::from(index),
            },
        );
    }

    let entry = GovernanceEntry::ExpressionAdded {
        expression_id: [99_u8; 16],
        name: "overflow".into(),
        kind: "emoji".into(),
        content_hash: "hash-overflow".into(),
        attachment: None,
        animated: false,
        tags: vec![],
        sound_meta: None,
        creator_pseudonym: None,
        created_at: None,
        available_to_peers: Some(true),
        lamport: 50,
    };
    assert!(!validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn expression_limit_rejects_extra_animated_emoji() {
    // Architecture §18.1 — separate 50-cap for animated; the
    // static count + animated count are independent, so a server
    // at the static limit can still accept animated uploads until
    // the animated bucket also fills.
    let mut state = state_with_creator_and_roles();
    for index in 0..50_u8 {
        state.expressions.insert(
            [index; 16],
            crate::state::ExpressionState {
                name: format!("anim-{index}"),
                kind: "emoji".into(),
                content_hash: format!("anim-hash-{index}"),
                attachment: None,
                animated: true,
                tags: vec![],
                sound_meta: None,
                creator_pseudonym: None,
                created_at: None,
                available_to_peers: true,
                lamport: u64::from(index),
            },
        );
    }

    let entry = GovernanceEntry::ExpressionAdded {
        expression_id: [200_u8; 16],
        name: "anim-overflow".into(),
        kind: "emoji".into(),
        content_hash: "anim-hash-overflow".into(),
        attachment: None,
        animated: true,
        tags: vec![],
        sound_meta: None,
        creator_pseudonym: None,
        created_at: None,
        available_to_peers: Some(true),
        lamport: 100,
    };
    assert!(
        !validate_write(&pseudo(5), &entry, &state),
        "51st animated emoji must be rejected"
    );

    // Sanity: a static-emoji upload should still succeed at the
    // animated cap because the buckets are independent.
    let static_entry = GovernanceEntry::ExpressionAdded {
        expression_id: [201_u8; 16],
        name: "still-fine".into(),
        kind: "emoji".into(),
        content_hash: "still-fine-hash".into(),
        attachment: None,
        animated: false,
        tags: vec![],
        sound_meta: None,
        creator_pseudonym: None,
        created_at: None,
        available_to_peers: Some(true),
        lamport: 101,
    };
    assert!(
        validate_write(&pseudo(5), &static_entry, &state),
        "static slot still has room when animated bucket is full"
    );
}

#[test]
fn soundboard_entry_with_valid_meta_accepted() {
    use rekindle_types::expression::SoundboardMeta;
    let state = state_with_creator_and_roles();
    let entry = GovernanceEntry::ExpressionAdded {
        expression_id: [10_u8; 16],
        name: "horn".into(),
        kind: "soundboard".into(),
        content_hash: "h".into(),
        attachment: None,
        animated: false,
        tags: vec![],
        sound_meta: Some(SoundboardMeta {
            duration_seconds: 2.5,
            volume: 0.8,
            emoji: Some("📯".into()),
        }),
        creator_pseudonym: None,
        created_at: None,
        available_to_peers: Some(true),
        lamport: 5,
    };
    assert!(validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn soundboard_entry_rejects_overlong_duration() {
    use rekindle_types::expression::SoundboardMeta;
    let state = state_with_creator_and_roles();
    let entry = GovernanceEntry::ExpressionAdded {
        expression_id: [11_u8; 16],
        name: "ten_sec".into(),
        kind: "soundboard".into(),
        content_hash: "h".into(),
        attachment: None,
        animated: false,
        tags: vec![],
        sound_meta: Some(SoundboardMeta {
            duration_seconds: 10.0,
            volume: 1.0,
            emoji: None,
        }),
        creator_pseudonym: None,
        created_at: None,
        available_to_peers: Some(true),
        lamport: 5,
    };
    assert!(!validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn soundboard_entry_rejects_volume_outside_range() {
    use rekindle_types::expression::SoundboardMeta;
    let state = state_with_creator_and_roles();
    let entry = GovernanceEntry::ExpressionAdded {
        expression_id: [12_u8; 16],
        name: "loud".into(),
        kind: "soundboard".into(),
        content_hash: "h".into(),
        attachment: None,
        animated: false,
        tags: vec![],
        sound_meta: Some(SoundboardMeta {
            duration_seconds: 1.0,
            volume: 1.5,
            emoji: None,
        }),
        creator_pseudonym: None,
        created_at: None,
        available_to_peers: Some(true),
        lamport: 5,
    };
    assert!(!validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn emoji_entry_rejects_smuggled_sound_meta() {
    use rekindle_types::expression::SoundboardMeta;
    let state = state_with_creator_and_roles();
    let entry = GovernanceEntry::ExpressionAdded {
        expression_id: [13_u8; 16],
        name: "weird".into(),
        kind: "emoji".into(),
        content_hash: "h".into(),
        attachment: None,
        animated: false,
        tags: vec![],
        sound_meta: Some(SoundboardMeta {
            duration_seconds: 1.0,
            volume: 1.0,
            emoji: None,
        }),
        creator_pseudonym: None,
        created_at: None,
        available_to_peers: Some(true),
        lamport: 5,
    };
    assert!(!validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn onboarding_rejects_too_many_questions() {
    use rekindle_types::governance::{OnboardingOption, OnboardingQuestion};
    let state = state_with_creator_and_roles();
    let mut questions = Vec::new();
    for i in 0..6_u8 {
        questions.push(OnboardingQuestion {
            question_id: format!("q{i}"),
            title: "x".into(),
            description: None,
            required: false,
            single_select: true,
            options: vec![OnboardingOption {
                option_id: "o".into(),
                title: "o".into(),
                description: None,
                emoji: None,
                roles_to_assign: vec![],
                channels_to_show: vec![],
            }],
        });
    }
    let entry = GovernanceEntry::OnboardingConfig {
        enabled: true,
        mode: "guided".into(),
        default_channels: vec![],
        questions,
        welcome_message: None,
        guide_steps: vec![],
        lamport: 10,
    };
    assert!(!validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn onboarding_rejects_overlong_welcome_message() {
    let state = state_with_creator_and_roles();
    let entry = GovernanceEntry::OnboardingConfig {
        enabled: true,
        mode: "default".into(),
        default_channels: vec![],
        questions: vec![],
        welcome_message: Some("a".repeat(501)),
        guide_steps: vec![],
        lamport: 10,
    };
    assert!(!validate_write(&pseudo(5), &entry, &state));
}

#[test]
fn onboarding_rejects_unknown_mode() {
    let state = state_with_creator_and_roles();
    let entry = GovernanceEntry::OnboardingConfig {
        enabled: true,
        mode: "bogus".into(),
        default_channels: vec![],
        questions: vec![],
        welcome_message: None,
        guide_steps: vec![],
        lamport: 10,
    };
    assert!(!validate_write(&pseudo(5), &entry, &state));
}

mod hierarchy;
