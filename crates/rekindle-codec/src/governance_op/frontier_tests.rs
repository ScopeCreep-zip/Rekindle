//! Frontier wire-format tests (`M`/3): a full frontier fits one subkey,
//! one head more is refused, and the roster checksum (D15) is required.

use rekindle_types::governance::{OpHash, OpRef};

use super::tests::pk;

#[test]
fn a_full_frontier_round_trips_under_the_subkey_cap() {
    use rekindle_types::dht_layout::member_record::{MAX_FRONTIER_REFS, MAX_SUBKEY_VALUE_LEN};
    let heads: Vec<OpRef> = (0..MAX_FRONTIER_REFS)
        .map(|i| OpRef {
            op_hash: OpHash([0xA5; 32]),
            device_key: pk(u8::try_from(i).unwrap()).0,
            author: pk(u8::try_from(i).unwrap()),
            author_record: "VLD0:".to_string() + &"x".repeat(43) + ":" + &"y".repeat(43),
            author_seq: u64::MAX - i as u64,
        })
        .collect();
    let value = rekindle_types::governance::Frontier {
        fork_proof: Vec::new(),
        fork_refs: Vec::new(),
        heads: heads.clone(),
        roster_checksum: rekindle_types::governance::RosterChecksum([0x3C; 32]),
    };
    let bytes = super::encode_frontier(&value).unwrap();
    assert!(bytes.len() <= MAX_SUBKEY_VALUE_LEN);
    assert_eq!(super::decode_frontier(&bytes).unwrap(), value);
    let mut over = value.clone();
    over.heads.push(heads[0].clone());
    assert!(super::encode_frontier(&over).is_err());
}

/// The roster checksum (D15) rides the frontier at its fresh ordinal and
/// comes back byte for byte; a frontier without one is refused.
#[test]
fn a_frontier_carries_its_roster_checksum() {
    use rekindle_types::governance::{Frontier, RosterChecksum};
    let value = Frontier {
        fork_proof: Vec::new(),
        fork_refs: Vec::new(),
        heads: vec![OpRef {
            op_hash: OpHash([1; 32]),
            device_key: pk(2).0,
            author: pk(2),
            author_record: "VLD0:m".into(),
            author_seq: 3,
        }],
        roster_checksum: RosterChecksum(std::array::from_fn(|i| u8::try_from(i).unwrap())),
    };
    let bytes = super::encode_frontier(&value).unwrap();
    assert_eq!(super::decode_frontier(&bytes).unwrap(), value);
    let mut builder = capnp::message::Builder::new_default();
    builder
        .init_root::<crate::governance_op_capnp::frontier::Builder<'_>>()
        .init_heads(0);
    let unsummed = crate::capnp_codec::pack(&builder);
    assert!(super::decode_frontier(&unsummed).is_err());
}
