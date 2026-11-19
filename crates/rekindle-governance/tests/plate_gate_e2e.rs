//! End-to-end Plate Gate scenario test (architecture §15).
//!
//! Verifies the full multi-segment fan-out at the governance layer:
//!
//!   1. Genesis admin creates a community + a channel.
//!   2. Admin writes `SegmentAdded` for segment 1 (architecture §15.2).
//!   3. The merged `GovernanceState` exposes the segment-1 registry and
//!      anchor keys in `segments`, and the admin's writes validate. A
//!      channel has no record in any segment (plan E3.3): posts ride each
//!      author's `M`/4 outbox, so no segment links a channel record.
//!
//! This is the in-process counterpart to the property test
//! `segments_converge_regardless_of_order` — that one fuzzes ordering;
//! this one asserts the concrete fan-out shape Plate Gate must produce.

use rekindle_governance::fixture::{at, merge_entries, Stamped};
use rekindle_governance::validate::validate_write;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::permissions::{ADMINISTRATOR, SEND_MESSAGES, VIEW_CHANNELS};

fn admin_pseudonym() -> PseudonymKey {
    PseudonymKey([0xAA; 32])
}

fn joiner_pseudonym() -> PseudonymKey {
    PseudonymKey([0xBB; 32])
}

fn channel_id() -> ChannelId {
    ChannelId([0xC1; 16])
}

fn admin_role_id() -> RoleId {
    RoleId([0x11; 16])
}

fn member_role_id() -> RoleId {
    RoleId([0x00; 16])
}

fn admin_entries() -> Vec<Stamped> {
    vec![
        at(
            1,
            GovernanceEntry::CommunityMeta {
                name: Some("plate-gate-e2e".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
            },
        ),
        // Two roles: admin (full perms) and everyone (send + view).
        at(
            2,
            GovernanceEntry::RoleDefinition {
                role_id: admin_role_id(),
                name: "admin".into(),
                permissions: ADMINISTRATOR,
                position: 1,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
            },
        ),
        at(
            3,
            GovernanceEntry::RoleDefinition {
                role_id: member_role_id(),
                name: "everyone".into(),
                permissions: VIEW_CHANNELS | SEND_MESSAGES,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
            },
        ),
        at(
            4,
            GovernanceEntry::RoleAssignment {
                target: admin_pseudonym(),
                role_id: admin_role_id(),
            },
        ),
        at(
            5,
            GovernanceEntry::RoleAssignment {
                target: joiner_pseudonym(),
                role_id: member_role_id(),
            },
        ),
        at(
            6,
            GovernanceEntry::ChannelCreated {
                channel_id: channel_id(),
                name: "general".into(),
                channel_type: "text".into(),
                category_id: None,
                position: 0,
                parent_voice_channel_id: None,
            },
        ),
        // Architecture §15.2: admin writes the segment expansion entry.
        at(
            7,
            GovernanceEntry::SegmentAdded {
                segment_index: 1,
                registry_key: "VLD0:REG-1".into(),
                anchor: "VLD0:GOV-1".into(),
                slot_range_start: 255,
                slot_range_end: 510,
            },
        ),
    ]
}

#[test]
fn segment1_record_appears_in_state_and_validates() {
    let state = merge_entries(
        &admin_pseudonym(),
        &[
            (admin_pseudonym(), admin_entries()),
        ],
    );

    // Segment 1's expansion landed.
    assert_eq!(state.segments.len(), 1);
    assert_eq!(state.segments[0].segment_index, 1);
    assert_eq!(state.segments[0].registry_key, "VLD0:REG-1");
    assert_eq!(state.segments[0].anchor, "VLD0:GOV-1");

    assert!(state.channels.contains_key(&channel_id()));

    // Reader-validate: the admin (creator) passes against the merged state.
    assert!(validate_write(
        &admin_pseudonym(),
        &admin_entries().last().unwrap().1,
        &state
    ));
}

