//! Test modules for rekindle-transport.
//!
//! Tests that require a live Veilid node are gated behind integration
//! test infrastructure. Unit tests here cover payload serialization.
//! Frame, envelope and voice crypto tests are inline in their
//! respective modules. Gossip moved to `tests/gossip_wire_compat.rs`
//! when the postcard `GossipPayload` was deleted — it tests the Cap'n
//! Proto envelope both tracks actually exchange.

pub mod mock_node;

use crate::payload::rpc::*;

#[test]
fn call_response_roundtrip() {
    let resp = CallResponse::Ok(b"response data".to_vec());
    let bytes = serialize_call_response(&resp);
    let back: CallResponse = postcard::from_bytes(&bytes).unwrap();
    match back {
        CallResponse::Ok(data) => assert_eq!(data, b"response data"),
        _ => panic!("wrong variant"),
    }
}
