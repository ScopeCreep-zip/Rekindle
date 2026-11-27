use rekindle_records::port::PortError;

use super::ProtocolError;

/// A route-unusable failure keeps its class both ways: the importer
/// must still forget the route.
#[test]
fn route_unusable_survives_the_port_round_trip() {
    let port = PortError::from(ProtocolError::RouteUnusable("no connection".into()));
    assert_eq!(port, PortError::RouteUnusable("no connection".into()));
    assert!(matches!(
        ProtocolError::from(port),
        ProtocolError::RouteUnusable(m) if m == "no connection"
    ));
}

#[test]
fn oversize_encoding_keeps_its_typed_refusal_through_the_record_port() {
    let error = ProtocolError::from(rekindle_codec::CodecError::TooLargeToEncode(700_000));
    let port = PortError::from(error);
    assert_eq!(port, PortError::TooLargeToEncode(700_000));
    assert!(matches!(
        ProtocolError::from(port),
        ProtocolError::TooLargeToEncode(700_000)
    ));
}
