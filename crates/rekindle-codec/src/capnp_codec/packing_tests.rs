use proptest::prelude::*;

use super::pack;

/// A message of `items` data elements, its first segment `first_words`
/// long so larger inputs spill into more segments.
fn message(
    items: &[Vec<u8>],
    first_words: u32,
) -> capnp::message::Builder<capnp::message::HeapAllocator> {
    let mut builder = capnp::message::Builder::new(
        capnp::message::HeapAllocator::new().first_segment_words(first_words),
    );
    let mut list = builder
        .initn_root::<capnp::data_list::Builder<'_>>(u32::try_from(items.len()).unwrap());
    for (i, item) in (0u32..).zip(items) {
        list.set(i, item);
    }
    builder
}

fn capnp_packed(builder: &capnp::message::Builder<capnp::message::HeapAllocator>) -> Vec<u8> {
    let mut out = Vec::new();
    capnp::serialize_packed::write_message(&mut out, builder).unwrap();
    out
}

/// Words that exercise every packing branch: zero runs past 255, literal
/// runs past 255, mixed words.
fn word_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        proptest::collection::vec(any::<u8>(), 0..300),
        proptest::collection::vec(1u8..=255, 0..4200),
        (0usize..4200).prop_map(|n| vec![0; n]),
    ]
}

proptest! {
    #[test]
    fn pack_matches_capnps_packer(
        items in proptest::collection::vec(word_bytes(), 0..6),
        first_words in 1u32..64,
    ) {
        let builder = message(&items, first_words);
        prop_assert_eq!(pack(&builder), capnp_packed(&builder));
    }
}

#[test]
fn a_multi_segment_message_packs_as_capnp_does() {
    let items = vec![vec![7u8; 3000], vec![0u8; 3000], vec![1, 0, 2, 0, 3]];
    let builder = message(&items, 1);
    assert!(builder.get_segments_for_output().len() > 1);
    assert_eq!(pack(&builder), capnp_packed(&builder));
}
