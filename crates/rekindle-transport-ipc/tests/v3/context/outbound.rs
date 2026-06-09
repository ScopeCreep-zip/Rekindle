use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::wire::frame_kind::{ChannelKind, StreamKind};

#[test]
fn push_channel_frame() {
    let (mut ctx, router) = make_test_context();
    ctx.push_outbound(OutboundFrame::Channel {
        kind: ChannelKind::Pong, payload: vec![0; 24],
    });
    let out = ctx.drain_outbound();
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0], OutboundFrame::Channel { kind: ChannelKind::Pong, .. }));
    assert_no_router_deliveries(&router);
}

#[test]
fn push_data_frame() {
    let (mut ctx, router) = make_test_context();
    ctx.push_outbound(OutboundFrame::Data {
        stream_id: 5, kind: StreamKind::Ack, chunk_index: 0, payload: vec![0; 64],
    });
    let out = ctx.drain_outbound();
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0], OutboundFrame::Data { stream_id: 5, .. }));
    assert_no_router_deliveries(&router);
}

#[test]
fn drain_clears_queue() {
    let (mut ctx, router) = make_test_context();
    ctx.push_outbound(OutboundFrame::Channel { kind: ChannelKind::Ping, payload: vec![] });
    ctx.push_outbound(OutboundFrame::Channel { kind: ChannelKind::Pong, payload: vec![] });
    let first = ctx.drain_outbound();
    assert_eq!(first.len(), 2);
    let second = ctx.drain_outbound();
    assert!(second.is_empty());
    assert_no_router_deliveries(&router);
}

#[test]
fn multiple_frames_preserve_order() {
    let (mut ctx, router) = make_test_context();
    let kinds = [
        ChannelKind::Hello, ChannelKind::HelloAck, ChannelKind::Goodbye,
        ChannelKind::GoodbyeAck, ChannelKind::Ping, ChannelKind::Pong,
        ChannelKind::Ack, ChannelKind::Nack, ChannelKind::Credit,
        ChannelKind::Backpressure,
    ];
    for kind in &kinds {
        ctx.push_outbound(OutboundFrame::Channel { kind: *kind, payload: vec![] });
    }
    let out = ctx.drain_outbound();
    assert_eq!(out.len(), 10);
    for (i, frame) in out.iter().enumerate() {
        match frame {
            OutboundFrame::Channel { kind, .. } => assert_eq!(*kind, kinds[i]),
            _ => panic!("expected Channel frame at index {i}"),
        }
    }
    assert_no_router_deliveries(&router);
}
