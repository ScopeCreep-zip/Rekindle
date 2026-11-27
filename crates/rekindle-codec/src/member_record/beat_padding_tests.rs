use super::*;
use rekindle_utils::random::OsRandom;

fn pack_list(bytes: &[u8]) -> usize {
    let mut builder = capnp::message::Builder::new_default();
    builder.set_root::<capnp::data::Owned>(bytes).unwrap();
    crate::capnp_codec::pack(&builder).unwrap().len()
}

/// The padding's packed cost, measured against a list of the same
/// length (so the same header and list pointer) whose every word holds
/// one nonzero byte, which packs to 2 bytes a word.
fn packed_cost(padding: &[u8]) -> usize {
    let mut reference = vec![0u8; padding.len()];
    for word in reference.chunks_exact_mut(8) {
        word[0] = 0x5a;
    }
    pack_list(padding) + 2 * (padding.len() / 8) - pack_list(&reference)
}

#[test]
fn every_padding_packs_to_exactly_its_cost() {
    for cost in MIN_COST..=400 {
        let pad = padding(cost, &OsRandom);
        assert_eq!(pad.len() % 8, 0);
        assert_eq!(packed_cost(&pad), cost, "cost {cost}");
    }
}
