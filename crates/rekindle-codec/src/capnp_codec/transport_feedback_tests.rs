use super::*;
use crate::capnp_codec::SignedWire;
use rekindle_secrets::ed25519_dalek::SigningKey;

fn signed() -> TransportFeedback {
    let key = SigningKey::from_bytes(&[9u8; 32]);
    let mut fb = TransportFeedback {
        reporter_key: key.verifying_key().to_bytes().to_vec(),
        begin_seq: 4_000_000_000,
        report_time_ms: 123_456,
        arrivals: vec![10, NOT_RECEIVED, 0, MAX_ARRIVAL_OFFSET],
        sig: Vec::new(),
    };
    fb.sign(&key);
    fb
}

#[test]
fn round_trip_and_verify() {
    let fb = signed();
    let decoded = TransportFeedback::decode(&fb.encode()).unwrap();
    assert_eq!(decoded, fb);
    decoded.verify().unwrap();
}

/// Every field is signature-covered: a relay rewriting an arrival
/// cannot steer the sender's estimate.
#[test]
fn tampering_breaks_the_signature() {
    let mut fb = signed();
    fb.arrivals[1] = 5;
    assert!(fb.verify().is_err());
    let mut fb = signed();
    fb.begin_seq += 1;
    assert!(fb.verify().is_err());
}
