use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::channel::error;
use rekindle_transport_ipc::v3::session::state::SessionState;

fn encode_error(code: u16, msg: &str) -> Vec<u8> {
    let msg_bytes = msg.as_bytes();
    let mut buf = Vec::with_capacity(8 + msg_bytes.len());
    buf.extend_from_slice(&code.to_le_bytes());
    buf.extend_from_slice(&[0, 0]);
    buf.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(msg_bytes);
    buf
}

#[test]
fn error_transitions_to_closed() {
    let (mut ctx, router) = make_test_context();
    error::handle(&mut ctx, &encode_error(0x0001, "test error")).unwrap();
    assert_eq!(ctx.session_state(), SessionState::Closed);
    assert_no_router_deliveries(&router);
}

#[test]
fn error_with_empty_message() {
    let (mut ctx, router) = make_test_context();
    error::handle(&mut ctx, &encode_error(0x0002, "")).unwrap();
    assert_eq!(ctx.session_state(), SessionState::Closed);
    assert_no_router_deliveries(&router);
}
