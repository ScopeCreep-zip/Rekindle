//! `outbox.rs`'s payload round trip, moved out to keep `outbox.rs` under the
//! 600-line ceiling.

use rekindle_codec::capnp_codec::direct::DmText;

use super::*;

#[test]
fn a_payload_round_trips_its_body() {
    let body = DirectBody::DmText(DmText {
        client_id: [7; 16],
        body: "hi".into(),
        reply_to: None,
        expire_seconds: 0,
    });
    assert_eq!(decode(&payload(&body).unwrap()).unwrap(), body);
    assert!(decode(b"{}").is_err());
}
