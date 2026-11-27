//! The protocol adapters parse untrusted record keys and keypairs.

use proptest::prelude::*;
use rekindle_protocol::dht::{backend::parse_keypair, parse_record_key};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    #[test]
    fn peer_key_text_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let text = String::from_utf8_lossy(&bytes);
        let _ = parse_record_key(&text);
        let _ = parse_keypair(&text);
    }
}
