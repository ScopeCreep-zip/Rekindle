use super::*;
use rekindle_types::governance::GovernanceSubkeyPayload;
use rekindle_types::id::{PseudonymKey, RoleId};

fn meta(lamport: u64) -> GovernanceEntry {
    GovernanceEntry::CommunityMeta {
        name: Some("Test".into()),
        description: None,
        icon_hash: None,
        banner_hash: None,
        lamport,
    }
}

fn chan_created(id: u8, lamport: u64) -> GovernanceEntry {
    GovernanceEntry::ChannelCreated {
        channel_id: ChannelId([id; 16]),
        name: "general".into(),
        channel_type: "text".into(),
        record_key: "VLD0:channelrecordkeyplaceholderplaceholder".into(),
        category_id: None,
        position: 0,
        parent_voice_channel_id: None,
        lamport,
    }
}

fn chan_updated(id: u8, lamport: u64) -> GovernanceEntry {
    GovernanceEntry::ChannelUpdated {
        channel_id: ChannelId([id; 16]),
        name: Some("renamed".into()),
        topic: None,
        forum_tags: None,
        position: None,
        slowmode_seconds: None,
        nsfw: None,
        category_id: None,
        lamport,
    }
}

fn chan_archived(id: u8, lamport: u64) -> GovernanceEntry {
    GovernanceEntry::ChannelArchived {
        channel_id: ChannelId([id; 16]),
        lamport,
    }
}

fn role_def(id: u8, lamport: u64) -> GovernanceEntry {
    GovernanceEntry::RoleDefinition {
        role_id: RoleId([id; 16]),
        name: "mods".into(),
        permissions: 0,
        position: 0,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
        lamport,
    }
}

fn role_archived(id: u8, lamport: u64) -> GovernanceEntry {
    GovernanceEntry::RoleArchived {
        role_id: RoleId([id; 16]),
        lamport,
    }
}

fn mek(generation: u64, lamport: u64) -> GovernanceEntry {
    GovernanceEntry::MEKGenerationBump {
        generation,
        trigger_departed: PseudonymKey([0u8; 32]),
        cascade_skipped: Vec::new(),
        lamport,
    }
}

fn invite_created(id: u8, lamport: u64) -> GovernanceEntry {
    GovernanceEntry::InviteCreated {
        invite_id: [id; 16],
        code_hash: "0".repeat(64),
        max_uses: 0,
        expires_at: None,
        secrets_record_key: "VLD0:invitesecretsrecordkeyplaceholderplc".into(),
        lamport,
    }
}

fn invite_created_exp(id: u8, lamport: u64, expires_at: Option<u64>) -> GovernanceEntry {
    GovernanceEntry::InviteCreated {
        invite_id: [id; 16],
        code_hash: "0".repeat(64),
        max_uses: 0,
        expires_at,
        secrets_record_key: "VLD0:invitesecretsrecordkeyplaceholderplc".into(),
        lamport,
    }
}

fn invite_revoked(id: u8, lamport: u64) -> GovernanceEntry {
    GovernanceEntry::InviteRevoked {
        invite_id: [id; 16],
        lamport,
    }
}

fn count<F: Fn(&GovernanceEntry) -> bool>(entries: &[GovernanceEntry], pred: F) -> usize {
    entries.iter().filter(|e| pred(e)).count()
}

#[test]
fn single_entry_is_returned_unchanged() {
    let out = compact_author_entries(vec![meta(1)], 0);
    assert_eq!(out.len(), 1);
}

#[test]
fn channel_create_and_archive_drop_both() {
    let out = compact_author_entries(
        vec![meta(1), chan_created(0xAA, 2), chan_archived(0xAA, 3)],
        0,
    );
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0], GovernanceEntry::CommunityMeta { .. }));
}

#[test]
fn updates_for_dead_channel_are_dropped() {
    let out = compact_author_entries(
        vec![
            meta(1),
            chan_created(0xAA, 2),
            chan_updated(0xAA, 3),
            chan_archived(0xAA, 4),
        ],
        0,
    );
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0], GovernanceEntry::CommunityMeta { .. }));
}

#[test]
fn live_channel_with_two_updates_is_kept() {
    let out = compact_author_entries(
        vec![
            meta(1),
            chan_created(0xAA, 2),
            chan_updated(0xAA, 3),
            chan_updated(0xAA, 4),
        ],
        0,
    );
    assert_eq!(out.len(), 4);
    assert_eq!(
        count(&out, |e| matches!(
            e,
            GovernanceEntry::ChannelUpdated { .. }
        )),
        2
    );
}

#[test]
fn roles_are_kept_verbatim() {
    // Conservative deviation from the §4.2 plan: roles feed `validate_write`
    // (cross-author moderation hierarchy + assignment gating), so a same
    // -author define+archive pair is NOT folded.
    let out = compact_author_entries(vec![meta(1), role_def(0xBB, 2), role_archived(0xBB, 3)], 0);
    assert_eq!(out.len(), 3);
    assert_eq!(
        count(&out, |e| matches!(
            e,
            GovernanceEntry::RoleDefinition { .. }
        )),
        1
    );
    assert_eq!(
        count(&out, |e| matches!(e, GovernanceEntry::RoleArchived { .. })),
        1
    );
}

#[test]
fn community_meta_collapses_to_latest() {
    let out = compact_author_entries(vec![role_def(0xBB, 1), meta(5), meta(10)], 0);
    assert_eq!(out.len(), 2);
    let metas: Vec<u64> = out
        .iter()
        .filter_map(|e| match e {
            GovernanceEntry::CommunityMeta { lamport, .. } => Some(*lamport),
            _ => None,
        })
        .collect();
    assert_eq!(metas, vec![10]);
}

#[test]
fn genesis_community_meta_is_preserved_even_when_superseded() {
    // The author's lowest-lamport entry is force-kept (creator genesis),
    // so an LWW collapse can never drop it even though a later meta wins.
    let out = compact_author_entries(vec![meta(1), meta(10)], 0);
    assert_eq!(out.len(), 2);
    assert_eq!(
        count(&out, |e| matches!(e, GovernanceEntry::CommunityMeta { .. })),
        2
    );
}

#[test]
fn mek_keeps_highest_generation() {
    let out = compact_author_entries(vec![meta(1), mek(1, 5), mek(2, 6), mek(2, 7)], 0);
    let kept: Vec<(u64, u64)> = out
        .iter()
        .filter_map(|e| match e {
            GovernanceEntry::MEKGenerationBump {
                generation,
                lamport,
                ..
            } => Some((*generation, *lamport)),
            _ => None,
        })
        .collect();
    assert_eq!(kept, vec![(2, 7)]);
}

#[test]
fn invite_create_and_revoke_keeps_only_tombstone() {
    let out = compact_author_entries(
        vec![meta(1), invite_created(0xCC, 2), invite_revoked(0xCC, 3)],
        0,
    );
    assert_eq!(out.len(), 2);
    assert_eq!(
        count(&out, |e| matches!(e, GovernanceEntry::InviteCreated { .. })),
        0
    );
    assert_eq!(
        count(&out, |e| matches!(e, GovernanceEntry::InviteRevoked { .. })),
        1
    );
}

#[test]
fn live_invite_is_kept() {
    let out = compact_author_entries(vec![meta(1), invite_created(0xCC, 2)], 0);
    assert_eq!(out.len(), 2);
    assert_eq!(
        count(&out, |e| matches!(e, GovernanceEntry::InviteCreated { .. })),
        1
    );
}

#[test]
fn expired_invite_is_pruned() {
    // expires_at(100) <= now(200) → dropped (no tombstone needed).
    let out = compact_author_entries(vec![meta(1), invite_created_exp(0xCC, 2, Some(100))], 200);
    assert_eq!(
        count(&out, |e| matches!(e, GovernanceEntry::InviteCreated { .. })),
        0
    );
}

#[test]
fn unexpired_invite_is_kept() {
    // expires_at(300) > now(200) → kept.
    let out = compact_author_entries(vec![meta(1), invite_created_exp(0xCC, 2, Some(300))], 200);
    assert_eq!(
        count(&out, |e| matches!(e, GovernanceEntry::InviteCreated { .. })),
        1
    );
}

#[test]
fn permanent_invite_is_kept() {
    // expires_at = None → never expires.
    let out = compact_author_entries(vec![meta(1), invite_created_exp(0xCC, 2, None)], u64::MAX);
    assert_eq!(
        count(&out, |e| matches!(e, GovernanceEntry::InviteCreated { .. })),
        1
    );
}

#[test]
fn over_cap_live_invites_keep_newest_n() {
    // meta(1) genesis + 10 permanent invites (lamports 2..=11). Cap is 8,
    // so the 8 newest survive and the 2 oldest (lamports 2, 3) are dropped.
    let mut entries = vec![meta(1)];
    for i in 0..10_u8 {
        entries.push(invite_created_exp(0x20 + i, u64::from(i) + 2, None));
    }
    let out = compact_author_entries(entries, 0);
    assert_eq!(
        count(&out, |e| matches!(e, GovernanceEntry::InviteCreated { .. })),
        MAX_ACTIVE_INVITES_PER_INVITER as usize
    );
    // The two oldest (lamports 2 and 3) are the ones dropped.
    let kept_lamports: Vec<u64> = out
        .iter()
        .filter_map(|e| match e {
            GovernanceEntry::InviteCreated { lamport, .. } => Some(*lamport),
            _ => None,
        })
        .collect();
    assert!(!kept_lamports.contains(&2));
    assert!(!kept_lamports.contains(&3));
    assert!(kept_lamports.contains(&11));
}

#[test]
fn genesis_invite_never_pruned_even_if_expired() {
    // The author's lowest-lamport entry is the genesis; it is force-kept
    // even when it is an expired invite.
    let out = compact_author_entries(vec![invite_created_exp(0xCC, 1, Some(100)), meta(2)], 200);
    assert_eq!(
        count(&out, |e| matches!(e, GovernanceEntry::InviteCreated { .. })),
        1
    );
}

/// Regression for the original bug: a creator subkey wedged at the ~4112 B
/// SMPL cap by three dead channel create+archive pairs. Appending the
/// `InviteCreated` pointer that overflowed the raw log must now fit once the
/// dead lifecycles are compacted away.
#[test]
fn wedged_subkey_self_heals_under_cap_when_invite_appended() {
    let mut entries = vec![
        meta(1),
        // three dead channel pairs (the ~1.3 KiB of removable weight)
        chan_created(0x10, 2),
        chan_archived(0x10, 3),
        chan_created(0x11, 4),
        chan_archived(0x11, 5),
        chan_created(0x12, 6),
        chan_archived(0x12, 7),
        // current state the creator still authors
        chan_created(0x20, 8),
        chan_created(0x21, 9),
        role_def(0x30, 10),
        role_def(0x31, 11),
        mek(1, 12),
        mek(2, 13),
        chan_updated(0x20, 14),
    ];
    // The write that previously overflowed: an invite pointer entry.
    entries.push(invite_created(0xEE, 15));

    let out = compact_author_entries(entries, 0);

    // All three dead channels (and their archives) are gone.
    assert_eq!(
        count(&out, |e| matches!(
            e,
            GovernanceEntry::ChannelArchived { .. }
        )),
        0
    );
    // The two live channels survive.
    assert_eq!(
        count(&out, |e| matches!(
            e,
            GovernanceEntry::ChannelCreated { .. }
        )),
        2
    );
    // Roles and the live invite are untouched.
    assert_eq!(
        count(&out, |e| matches!(
            e,
            GovernanceEntry::RoleDefinition { .. }
        )),
        2
    );
    assert_eq!(
        count(&out, |e| matches!(e, GovernanceEntry::InviteCreated { .. })),
        1
    );
    // Only the highest MEK generation remains.
    assert_eq!(
        count(&out, |e| matches!(
            e,
            GovernanceEntry::MEKGenerationBump { .. }
        )),
        1
    );

    let payload = GovernanceSubkeyPayload {
        author_pseudonym: PseudonymKey([7u8; 32]),
        entries: out,
        overflow_next: None,
        signature: vec![0u8; 64],
    };
    let bytes = serde_json::to_vec(&payload).expect("serialize payload");
    assert!(
        bytes.len() < 4112,
        "compacted payload {} B should fit under the SMPL per-subkey cap",
        bytes.len()
    );
}
