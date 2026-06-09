use rekindle_transport_ipc::v3::dispatch::inbound::dispatch_frame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{
    make_test_context, test_envelope, make_minimal_channel_payload,
    make_minimal_stream_payload, test_stream_header,
    make_minimal_datagram_payload, make_minimal_audit_payload,
    make_minimal_handoff_payload, assert_no_router_deliveries,
};
use rekindle_transport_ipc::v3::wire::frame_kind::*;
use rekindle_transport_ipc::v3::wire::lane::Lane;

#[test]
fn dispatch_covers_all_channel_kinds() {
    for &kind in ChannelKind::all_variants() {
        let (mut ctx, router) = make_test_context();
        let payload = make_minimal_channel_payload(kind);
        let result = dispatch_frame(&mut ctx, &test_envelope(Lane::Control), None, &payload);
        assert!(
            !matches!(result, Err(ref e) if format!("{e:?}").contains("UnknownFrame")),
            "dispatch has no arm for ChannelKind::{kind:?}"
        );
        assert_no_router_deliveries(&router);
    }
}

#[test]
fn dispatch_covers_all_stream_kinds() {
    for &kind in StreamKind::all_variants() {
        let (mut ctx, router) = make_test_context();
        let header = test_stream_header(kind);
        let payload = make_minimal_stream_payload(kind);
        let result = dispatch_frame(&mut ctx, &test_envelope(Lane::Data), Some(&header), &payload);
        assert!(
            !matches!(result, Err(ref e) if format!("{e:?}").contains("UnknownFrame")),
            "dispatch has no arm for StreamKind::{kind:?}"
        );
        assert_no_router_deliveries(&router);
    }
}

#[test]
fn dispatch_covers_all_datagram_kinds() {
    for &kind in DatagramKind::all_variants() {
        let (mut ctx, router) = make_test_context();
        let payload = make_minimal_datagram_payload(kind);
        let result = dispatch_frame(&mut ctx, &test_envelope(Lane::Control), None, &payload);
        assert!(
            !matches!(result, Err(ref e) if format!("{e:?}").contains("UnknownFrame")),
            "dispatch has no arm for DatagramKind::{kind:?}"
        );
        // Datagram handlers fail with CodecFailed on minimal payloads —
        // that's expected. The router should not have received any
        // deliveries because the codec failed before the router call.
        assert_no_router_deliveries(&router);
    }
}

#[test]
fn dispatch_covers_all_audit_kinds() {
    for &kind in AuditKind::all_variants() {
        let (mut ctx, router) = make_test_context();
        let payload = make_minimal_audit_payload(kind);
        let result = dispatch_frame(&mut ctx, &test_envelope(Lane::Audit), None, &payload);
        assert!(
            !matches!(result, Err(ref e) if format!("{e:?}").contains("UnknownFrame")),
            "dispatch has no arm for AuditKind::{kind:?}"
        );
        assert_no_router_deliveries(&router);
    }
}

#[test]
fn dispatch_covers_all_handoff_kinds() {
    for &kind in HandoffKind::all_variants() {
        let (mut ctx, router) = make_test_context();
        let payload = make_minimal_handoff_payload(kind);
        let result = dispatch_frame(&mut ctx, &test_envelope(Lane::Handoff), None, &payload);
        assert!(
            !matches!(result, Err(ref e) if format!("{e:?}").contains("UnknownFrame")),
            "dispatch has no arm for HandoffKind::{kind:?}"
        );
        assert_no_router_deliveries(&router);
    }
}
