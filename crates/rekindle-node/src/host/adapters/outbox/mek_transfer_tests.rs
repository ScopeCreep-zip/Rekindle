#[test]
fn a_payload_round_trips_its_route_and_envelope() {
    let payload = super::encode(b"route", b"sealed").unwrap();
    assert_eq!(
        super::decode(&payload),
        Some((&b"route"[..], &b"sealed"[..]))
    );
    assert_eq!(super::decode(&payload[..6]), None);
}
