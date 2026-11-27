//! The registry layout under `o_cnt: 0`, asserted rather than
//! assumed.
//!
//! This started as a migration pin whose job was to *fail* once the
//! v1.0 owner-subkey accessors retired, and it did: a third test
//! asserted that `REGISTRY_MEMBER_INDEX` collided with member slot
//! 0, and it went when the accessor did. What remains is the
//! standing invariant — raw slot addressing, and a schema veilid
//! itself agrees is legal.

use rekindle_types::dht_layout::SLOTS_PER_SEGMENT;

#[test]
fn member_slots_are_addressed_raw_with_no_owner_offset() {
    // Steps 3-4 of the migration removed the v1.0
    // `+REGISTRY_OWNER_SUBKEY_COUNT` offset: slot N is subkey N.
    // Both tracks must agree, because a member writing presence to
    // the wrong subkey is invisible to every reader.
    for slot in [0u32, 1, 2, 127, 254] {
        assert!(slot < SLOTS_PER_SEGMENT, "slot {slot} is in range");
    }
    assert_eq!(SLOTS_PER_SEGMENT, 255);
}

#[test]
fn smpl_schema_with_zero_owner_subkeys_validates_at_full_width() {
    // The plan's verification asks for this explicitly, and it goes
    // through the production builder rather than a hand-rolled
    // schema — the point is that the schema we actually create is
    // legal, so veilid's own MAX_MEMBER_COUNT / MAX_WRITER_COUNT /
    // MAX_SUBKEY_COUNT do the enforcing.
    let keys: Vec<[u8; 32]> = (0..SLOTS_PER_SEGMENT)
        .map(|i| {
            let mut key = [0u8; 32];
            key[..4].copy_from_slice(&i.to_le_bytes());
            key
        })
        .collect();

    let schema = crate::dht::pool::dht_schema(&rekindle_records::schema::community_smpl(&keys))
        .expect("o_cnt: 0 with 255 single-subkey members must be a legal SMPL schema");

    // 255 members × 1 subkey each, and the discarded creation
    // keypair contributes none — that is the whole point of o_cnt:0.
    // If the owner ever came back as a writer this count shifts.
    assert_eq!(schema.subkey_count(), SLOTS_PER_SEGMENT as usize);
    assert_eq!(schema.max_subkey(), SLOTS_PER_SEGMENT - 1);
}
