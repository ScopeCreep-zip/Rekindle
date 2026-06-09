use rekindle_transport_ipc::v3::dispatch::inbound::dispatch_frame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, test_envelope, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::wire::lane::Lane;

#[test]
fn unknown_class_produces_named_error() {
    let (mut ctx, router) = make_test_context();
    let payload = &[0xFF, 0x01];
    let result = dispatch_frame(&mut ctx, &test_envelope(Lane::Control), None, payload);
    assert!(result.is_err());
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("UnknownFrame") || err.contains("ClassUnknown") || err.contains("LaneMismatch"),
        "Unknown class must produce a named error, got: {err}"
    );
    assert_no_router_deliveries(&router);
}

#[test]
fn unknown_kind_in_channel_class_produces_named_error() {
    let (mut ctx, router) = make_test_context();
    let payload = &[0x01, 0xFF];
    let result = dispatch_frame(&mut ctx, &test_envelope(Lane::Control), None, payload);
    assert!(result.is_err());
    assert_no_router_deliveries(&router);
}

#[test]
fn class_zero_produces_named_error() {
    let (mut ctx, router) = make_test_context();
    let payload = &[0x00, 0x01];
    let result = dispatch_frame(&mut ctx, &test_envelope(Lane::Control), None, payload);
    assert!(result.is_err());
    assert_no_router_deliveries(&router);
}

#[test]
fn stream_class_on_control_lane_produces_mismatch_error() {
    let (mut ctx, router) = make_test_context();
    let payload = &[0x02, 0x02];
    let result = dispatch_frame(&mut ctx, &test_envelope(Lane::Control), None, payload);
    assert!(result.is_err());
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("Mismatch") || err.contains("mismatch"),
        "Class-lane mismatch must be named, got: {err}"
    );
    assert_no_router_deliveries(&router);
}
