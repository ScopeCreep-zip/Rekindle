//! The DHT value caps in `rekindle_types::dht_layout` are copies of
//! Veilid's: Tier 1 cannot link veilid-core (ADR 0014), so this crate, which
//! may, ties each copy to the pinned Veilid (plan VU6). An upstream change of
//! `MAX_SUBKEY_SIZE` or `MAX_RECORD_DATA_SIZE` (veilid-core 0.5.8
//! `veilid_api/types/dht/mod.rs:36,38`) fails here instead of on a peer's
//! refused write.

use rekindle_types::dht_layout::{self, member_record, prekeys, request_inbox};

/// Veilid's per-subkey cap for a record whose limit divides the data cap by
/// `divisor`: `o_cnt` for DFLT (`derivation.rs:145-148`), the subkey count
/// for SMPL (`derivation.rs:186-189`).
fn veilid_cap(divisor: u16) -> usize {
    usize::min(
        veilid_core::MAX_SUBKEY_SIZE,
        veilid_core::MAX_RECORD_DATA_SIZE / usize::from(divisor),
    )
}

#[test]
fn subkey_cap_is_veilids() {
    assert_eq!(dht_layout::MAX_SUBKEY_SIZE, veilid_core::MAX_SUBKEY_SIZE);
}

#[test]
fn per_record_caps_are_veilids() {
    // DFLT(101): the divisor is o_cnt, every subkey.
    assert_eq!(
        prekeys::MAX_SUBKEY_VALUE_LEN,
        veilid_cap(prekeys::SUBKEY_COUNT)
    );
    // DFLT(8).
    assert_eq!(
        member_record::MAX_SUBKEY_VALUE_LEN,
        veilid_cap(member_record::SUBKEY_COUNT)
    );
    // SMPL(o_cnt 1, one writer subkey): the divisor is the subkey count.
    assert_eq!(
        request_inbox::MAX_SUBKEY_VALUE_LEN,
        veilid_cap(request_inbox::SUBKEY_COUNT)
    );
}
