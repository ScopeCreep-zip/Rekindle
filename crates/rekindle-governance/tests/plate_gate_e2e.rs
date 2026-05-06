//! End-to-end Plate Gate scenario test (architecture §15).
//!
//! Verifies the full multi-segment fan-out at the governance layer:
//!
//!   1. Genesis admin creates a community + a channel.
//!   2. Admin writes `SegmentAdded` for segment 1 (architecture §15.2).
//!   3. A new joiner appears in segment 1 and writes the very first
//!      message there → emits `ChannelSegmentLinked`
//!      (architecture §15.4 lazy channel record).
//!   4. Both members' entries are merged in arbitrary order; the
//!      resulting `GovernanceState` exposes:
//!        - the segment-1 registry+governance keys in `segments`
//!        - the segment-1 channel SMPL key in `channel_segment_records`
//!        - both members are valid writers (validate_write passes)
//!
//! This is the in-process counterpart to the property test
//! `segments_converge_regardless_of_order` — that one fuzzes ordering;
//! this one asserts the concrete fan-out shape Plate Gate must produce.

use rekindle_governance::{merge::merge, validate::validate_write};
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

fn admin_entries() -> Vec<GovernanceEntry> {
    vec![
        GovernanceEntry::CommunityMeta {
            name: Some("plate-gate-e2e".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        // Two roles: admin (full perms) and everyone (send + view).
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
            lamport: 2,
        },
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
            lamport: 3,
        },
        GovernanceEntry::RoleAssignment {
            target: admin_pseudonym(),
            role_id: admin_role_id(),
            lamport: 4,
        },
        GovernanceEntry::RoleAssignment {
            target: joiner_pseudonym(),
            role_id: member_role_id(),
            lamport: 5,
        },
        GovernanceEntry::ChannelCreated {
            channel_id: channel_id(),
            name: "general".into(),
            channel_type: "text".into(),
            record_key: "VLD0:CH-SEGMENT-0".into(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
            lamport: 6,
        },
        // Architecture §15.2: admin writes the segment expansion entry.
        GovernanceEntry::SegmentAdded {
            segment_index: 1,
            registry_key: "VLD0:REG-1".into(),
            governance_key: "VLD0:GOV-1".into(),
            slot_range_start: 255,
            slot_range_end: 510,
            lamport: 7,
        },
    ]
}

fn joiner_entries() -> Vec<GovernanceEntry> {
    vec![
        // Architecture §15.4: the first segment-N message in a channel
        // also emits a ChannelSegmentLinked governance entry so other
        // members open + watch the new SMPL record.
        GovernanceEntry::ChannelSegmentLinked {
            channel_id: channel_id(),
            segment_index: 1,
            record_key: "VLD0:CH-SEGMENT-1".into(),
            lamport: 10,
        },
    ]
}

#[test]
fn segment1_record_appears_in_state_and_validates() {
    let state = merge(&[
        (admin_pseudonym(), admin_entries()),
        (joiner_pseudonym(), joiner_entries()),
    ]);

    // Segment 1's expansion landed.
    assert_eq!(state.segments.len(), 1);
    assert_eq!(state.segments[0].segment_index, 1);
    assert_eq!(state.segments[0].registry_key, "VLD0:REG-1");
    assert_eq!(state.segments[0].governance_key, "VLD0:GOV-1");

    // Segment-1's lazy channel record landed under the right key.
    let record = state
        .channel_segment_records
        .get(&(channel_id(), 1))
        .expect("channel_segment_records must contain segment 1");
    assert_eq!(record.record_key, "VLD0:CH-SEGMENT-1");

    // Segment-0 channel record stays in `channels`, not in
    // `channel_segment_records` — segment 0 is the genesis path.
    assert!(state.channels.contains_key(&channel_id()));
    assert!(!state.channel_segment_records.contains_key(&(channel_id(), 0)));

    // Reader-validate: both writers should validate against the merged
    // state. Admin (creator) always passes; joiner has SEND_MESSAGES
    // via the everyone role (id 0) and the channel exists.
    assert!(validate_write(
        &admin_pseudonym(),
        admin_entries().last().unwrap(),
        &state
    ));
    assert!(validate_write(
        &joiner_pseudonym(),
        joiner_entries().last().unwrap(),
        &state
    ));
}

#[test]
fn channel_segment_linked_lww_resolves_first_writer_race() {
    // Race: two segment-1 writers both create the lazy channel record
    // and announce it. LWW per (channel_id, segment_index) picks the
    // higher-lamport entry; the loser's orphan SMPL record is never
    // referenced by any peer's governance state.
    let racer_a = PseudonymKey([0xCA; 32]);
    let racer_b = PseudonymKey([0xCB; 32]);
    let mut a_entries = admin_entries();
    a_entries.push(GovernanceEntry::ChannelSegmentLinked {
        channel_id: channel_id(),
        segment_index: 1,
        record_key: "VLD0:RACER-A".into(),
        lamport: 11,
    });
    let b_entries = vec![GovernanceEntry::ChannelSegmentLinked {
        channel_id: channel_id(),
        segment_index: 1,
        record_key: "VLD0:RACER-B".into(),
        lamport: 12,
    }];

    let state = merge(&[(admin_pseudonym(), a_entries), (racer_b.clone(), b_entries)]);
    let _ = racer_a;
    let record = state
        .channel_segment_records
        .get(&(channel_id(), 1))
        .expect("LWW must materialize one record");
    assert_eq!(record.record_key, "VLD0:RACER-B", "higher lamport wins");
    assert_eq!(record.linked_lamport, 12);
}
