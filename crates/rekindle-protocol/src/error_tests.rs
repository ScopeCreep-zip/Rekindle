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
