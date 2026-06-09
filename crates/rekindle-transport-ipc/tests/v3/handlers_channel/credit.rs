use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::channel::credit;
use rekindle_transport_ipc::v3::codec::channel::credit as credit_codec;

#[test]
fn lane_scoped_credit_updates_lane_budget() {
    let (mut ctx, router) = make_test_context();
    let payload = credit_codec::encode(&credit_codec::CreditPayload {
        scope: 0x01, lane_or_stream_id: 0x02, credit_bytes: 1_048_576, credit_frames: 64, credit_generation: 1,
    });
    credit::handle(&mut ctx, &payload).unwrap();
    assert!(ctx.lane_credit_bytes(0x02) >= 1_048_576);
    assert_no_router_deliveries(&router);
}

#[test]
fn stream_scoped_credit_updates_stream_tracker() {
    let (mut ctx, router) = make_test_context();
    ctx.stream_registry_mut().open(5).unwrap();
    ctx.create_reassembler(5);

    let payload = credit_codec::encode(&credit_codec::CreditPayload {
        scope: 0x02, lane_or_stream_id: 5, credit_bytes: 65536, credit_frames: 4, credit_generation: 1,
    });
    credit::handle(&mut ctx, &payload).unwrap();
    assert!(ctx.stream_credit_remaining(5) > 0);
    assert_no_router_deliveries(&router);
}

#[test]
fn credit_for_unknown_stream_ignored() {
    let (mut ctx, router) = make_test_context();
    let payload = credit_codec::encode(&credit_codec::CreditPayload {
        scope: 0x02, lane_or_stream_id: 99, credit_bytes: 65536, credit_frames: 4, credit_generation: 1,
    });
    let result = credit::handle(&mut ctx, &payload);
    assert!(result.is_ok());
    assert_no_router_deliveries(&router);
}
