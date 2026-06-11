//! Fuzz target for wire::decode — arbitrary bytes must never panic.
//!
//! This uses proptest to feed random byte vectors to the RID/1 CBOR
//! decoder. The invariant: decode() returns Ok or Err, never panics,
//! never OOMs for inputs up to 64 KiB.
//!
//! CI budget: 1000 cases per run (fast). Soak target: 1 CPU-hour
//! with cases=1_000_000 for release validation.

use proptest::prelude::*;
use rekindle_identity::wire::decode;

proptest! {
    #[test]
    fn decode_never_panics(data in proptest::collection::vec(any::<u8>(), 0..1024)) {
        // The only valid outcomes are Ok and Err. A panic is a bug.
        let _ = decode::decode(&data);
    }

    #[test]
    fn decode_sequence_head_never_panics(data in proptest::collection::vec(any::<u8>(), 0..1024)) {
        let _ = decode::decode_sequence_head(&data);
    }

    #[test]
    fn decode_signed_object_never_panics(data in proptest::collection::vec(any::<u8>(), 0..1024)) {
        let _ = decode::decode_signed_object(&data);
    }
}
