//! Tests for [`super::GovernanceEntry`] and [`super::GovernanceSubkeyPayload`].

use crate::id::{ChannelId, PseudonymKey, RoleId};

use super::*;

#[test]
fn governance_entry_serde_roundtrip() {
    let entry = GovernanceEntry::ChannelCreated {
        channel_id: ChannelId([1u8; 16]),
        name: "general".into(),
        channel_type: "text".into(),
        record_key: "VLD0:abc123".into(),
        category_id: None,
        position: 0,
        parent_voice_channel_id: None,
        lamport: 1,
    };
    let json = serde_json::to_string(&entry).unwrap();
    let back: GovernanceEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(entry, back);
}

#[test]
fn lamport_extraction() {
    let entry = GovernanceEntry::BanEntry {
        target: PseudonymKey([0xAA; 32]),
        reason: Some("spam".into()),
        lamport: 42,
    };
    assert_eq!(entry.lamport(), 42);
}

#[test]
fn all_variants_have_lamport() {
    // This test ensures the lamport() match is exhaustive.
    // If a new variant is added without a lamport field,
    // this will fail to compile.
    let entries = vec![
        GovernanceEntry::ChannelCreated {
            channel_id: ChannelId([0; 16]),
            name: String::new(),
            channel_type: String::new(),
            record_key: String::new(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
            lamport: 1,
        },
        GovernanceEntry::ChannelArchived {
            channel_id: ChannelId([0; 16]),
            lamport: 2,
        },
        GovernanceEntry::RoleDefinition {
            role_id: RoleId([0; 16]),
            name: String::new(),
            permissions: 0,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 3,
        },
        GovernanceEntry::MEKGenerationBump {
            generation: 1,
            trigger_departed: PseudonymKey([0; 32]),
            cascade_skipped: vec![],
            lamport: 4,
        },
    ];
    for e in &entries {
        assert!(e.lamport() > 0);
    }
}

#[test]
fn signing_bytes_uniform_v1_tag_and_none_pointer_appends_nothing() {
    // A non-overflow payload (overflow_next == None) must sign over exactly
    // the canonical governance form on the single `-v1` domain tag. This
    // locks the tag against a future drift (the dd90241 `-v2` bump orphaned
    // every pre-existing community's governance) and proves `None` appends
    // no pointer bytes, so existing `-v1` signatures keep verifying.
    let author = PseudonymKey([0x11; 32]);
    let entries = vec![GovernanceEntry::ChannelArchived {
        channel_id: ChannelId([0x22; 16]),
        lamport: 7,
    }];
    let payload = GovernanceSubkeyPayload {
        author_pseudonym: author.clone(),
        entries: entries.clone(),
        overflow_next: None,
        signature: vec![],
    };

    let entries_json = serde_json::to_vec(&entries).unwrap();
    let mut expected = Vec::new();
    expected.extend_from_slice(b"rekindle-gov-subkey-v1");
    expected.extend_from_slice(&author.0);
    expected.extend_from_slice(&(entries.len() as u64).to_le_bytes());
    expected.extend_from_slice(&entries_json);
    assert_eq!(payload.signing_bytes(), expected);

    // Setting the pointer appends its bytes (and only those) — binding it
    // into the signature without disturbing the None-case canonical form.
    let with_ptr = GovernanceSubkeyPayload {
        overflow_next: Some("VLD0:overflowkey".into()),
        ..payload
    };
    let mut expected_ptr = expected;
    expected_ptr.extend_from_slice(b"VLD0:overflowkey");
    assert_eq!(with_ptr.signing_bytes(), expected_ptr);
}
