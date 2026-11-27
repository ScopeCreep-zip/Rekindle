#[test]
fn a_payload_round_trips_its_hash_and_record() {
    let payload = super::encode(&[7; 32], "VLD0:rec");
    assert_eq!(super::decode(&payload), Some(([7; 32], "VLD0:rec")));
    assert_eq!(super::decode(&payload[..32]), None);
    assert_eq!(super::decode(&payload[..10]), None);
}

#[test]
fn a_dead_inviter_route_defers_the_row_without_a_call() {
    use super::{callable, AttemptOutcome};
    assert!(matches!(
        callable(Some(b"A".to_vec()), |b| b == b"A"),
        Err(AttemptOutcome::Deferred)
    ));
    assert!(matches!(
        callable(None, |_| false),
        Err(AttemptOutcome::Deferred)
    ));
    assert_eq!(
        callable(Some(b"B".to_vec()), |b| b == b"A").ok(),
        Some(b"B".to_vec())
    );
}
