use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::stream::resume;
use rekindle_transport_ipc::v3::codec::stream::resume as resume_codec;
use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

#[test]
fn resume_accepted_reopens_stream() {
    let (mut ctx, router) = make_test_context();
    let tid = uuid::Uuid::from_u128(42);
    let audit_link = [0xAA; 32];
    let content_hash = [0xBB; 32];
    ctx.register_resume_state(tid, 1_048_576, 64, audit_link, content_hash);

    let header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Resume,
        stream_id: 10, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    let payload = resume_codec::encode(&resume_codec::StreamResumePayload {
        transfer_id: tid, resume_from_byte: 1_048_576, resume_from_chunk: 64,
        anchor_audit_link: audit_link, anchor_content_hash: content_hash,
    });
    resume::handle(&mut ctx, &header, &payload).unwrap();

    assert_eq!(ctx.stream_active_count(), 1);
    assert!(ctx.has_reassembler(10));
    assert_no_router_deliveries(&router);
}

#[test]
fn resume_denied_produces_resume_deny() {
    let (mut ctx, router) = make_test_context();
    let tid = uuid::Uuid::from_u128(99);

    let header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Resume,
        stream_id: 10, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    let payload = resume_codec::encode(&resume_codec::StreamResumePayload {
        transfer_id: tid, resume_from_byte: 0, resume_from_chunk: 0,
        anchor_audit_link: [0; 32], anchor_content_hash: [0; 32],
    });
    let result = resume::handle(&mut ctx, &header, &payload);
    assert!(result.is_err());

    let out = ctx.drain_outbound();
    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Data { kind: StreamKind::ResumeDeny, .. })),
        "denied resume must produce RESUME_DENY"
    );
    assert_no_router_deliveries(&router);
}
