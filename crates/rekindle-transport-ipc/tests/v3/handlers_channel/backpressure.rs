use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::channel::backpressure;

fn encode_bp(resource: u8, severity: u8) -> Vec<u8> {
    let mut buf = vec![0u8; 16];
    buf[0] = resource;
    buf[1] = severity;
    buf[4..8].copy_from_slice(&1000u32.to_le_bytes());
    buf
}

#[test]
fn critical_backpressure_blocks() {
    let (mut ctx, router) = make_test_context();
    backpressure::handle_assert(&mut ctx, &encode_bp(0x01, 0x03)).unwrap();
    assert!(ctx.is_backpressured());
    assert_no_router_deliveries(&router);
}

#[test]
fn advisory_backpressure_does_not_block() {
    let (mut ctx, router) = make_test_context();
    backpressure::handle_assert(&mut ctx, &encode_bp(0x01, 0x01)).unwrap();
    assert!(!ctx.is_backpressured());
    assert_no_router_deliveries(&router);
}

#[test]
fn backpressure_clear_unblocks() {
    let (mut ctx, router) = make_test_context();
    backpressure::handle_assert(&mut ctx, &encode_bp(0x01, 0x03)).unwrap();
    assert!(ctx.is_backpressured());
    backpressure::handle_clear(&mut ctx, &encode_bp(0x01, 0x00));
    assert!(!ctx.is_backpressured());
    assert_no_router_deliveries(&router);
}
