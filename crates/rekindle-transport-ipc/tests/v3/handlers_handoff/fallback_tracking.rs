use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::handoff::reject;

#[test]
fn three_consecutive_rejections_disables_handoff() {
    let (mut ctx, router) = make_test_context();
    for i in 0..3 {
        let handoff_id = uuid::Uuid::from_u128(i);
        ctx.register_pending_handoff(handoff_id);
        let mut payload = Vec::new();
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(handoff_id.as_bytes());
        reject::handle(&mut ctx, &payload).unwrap();
    }
    assert!(ctx.fallback_tracker().is_disabled());
    assert_no_router_deliveries(&router);
}
