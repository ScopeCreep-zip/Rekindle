//! Tests for origin: the channel id, the architecture constants and the
//! genesis ops (moved out of `origin.rs`, file ceiling).

use super::*;
use rekindle_types::dht_layout::SLOTS_PER_SEGMENT;

#[test]
fn random_channel_id_is_16_bytes_and_random() {
    let a = random_channel_id(&rekindle_utils::OsRandom);
    let b = random_channel_id(&rekindle_utils::OsRandom);
    assert_ne!(a.0, b.0, "two consecutive channel ids should differ");
}

#[test]
fn constants_match_architecture_section_4_6() {
    assert_eq!(SLOTS_PER_SEGMENT, 255);
    assert_eq!(CREATOR_SLOT, 0);
    assert_eq!(OWNER_ROLE_ID.0, [0xFFu8; 16]);
}

/// No admission entry among the genesis ops: the cert pins the mode.
#[test]
fn genesis_ops_are_meta_roles_owner_and_general() {
    let entries = super::genesis_entries(
        "c",
        &rekindle_types::id::PseudonymKey([1u8; 32]),
        rekindle_types::id::ChannelId([2u8; 16]),
    );
    let kinds: Vec<&str> = entries
        .iter()
        .map(|e| match e {
            super::GovernanceEntry::CommunityMeta { .. } => "meta",
            super::GovernanceEntry::RoleDefinition { .. } => "role",
            super::GovernanceEntry::RoleAssignment { .. } => "assign",
            super::GovernanceEntry::ChannelCreated { .. } => "channel",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["meta", "role", "role", "assign", "channel"]);
    assert_eq!(super::GENESIS_LAST_SEQ, entries.len() as u64 - 1);
}
