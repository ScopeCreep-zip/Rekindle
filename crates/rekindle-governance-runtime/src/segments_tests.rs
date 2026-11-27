use super::*;

#[test]
fn slot_range_arithmetic() {
    // Segment 1 hosts slots 255..510, segment 2: 510..765 — matches
    // architecture §15.2 example.
    let segment_1_start = SLOTS_PER_SEGMENT;
    let segment_1_end = segment_1_start + SLOTS_PER_SEGMENT;
    assert_eq!(segment_1_start, 255);
    assert_eq!(segment_1_end, 510);

    let segment_2_start = 2 * SLOTS_PER_SEGMENT;
    let segment_2_end = segment_2_start + SLOTS_PER_SEGMENT;
    assert_eq!(segment_2_start, 510);
    assert_eq!(segment_2_end, 765);
}

#[test]
fn max_segments_default_is_eight() {
    // Capacity check: MAX_SEGMENTS * SLOTS_PER_SEGMENT = 2040 members.
    assert_eq!(MAX_SEGMENTS, 8);
    assert_eq!(MAX_SEGMENTS * SLOTS_PER_SEGMENT, 2040);
}

#[test]
fn slot_seed_from_membership_decodes_hex() {
    let membership = CommunityMembership {
        anchor: None,
        member_registry_key: None,
        my_pseudonym_hex: None,
        my_subkey_index: None,
        my_segment_index: None,
        slot_keypair: None,
        slot_seed_hex: Some(hex::encode([5u8; 32])),
        channel_ids: Vec::new(),
        mek_generation: 0,
    };
    let seed = slot_seed_from_membership(&membership, "test").expect("decode");
    assert_eq!(seed.0, [5u8; 32]);
}

#[test]
fn slot_seed_from_membership_missing_errors() {
    let membership = CommunityMembership {
        anchor: None,
        member_registry_key: None,
        my_pseudonym_hex: None,
        my_subkey_index: None,
        my_segment_index: None,
        slot_keypair: None,
        slot_seed_hex: None,
        channel_ids: Vec::new(),
        mek_generation: 0,
    };
    assert!(matches!(
        slot_seed_from_membership(&membership, "test"),
        Err(GovernanceRuntimeError::SlotSeedMissing(_))
    ));
}
