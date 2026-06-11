use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_bulk_delivered};
use rekindle_transport_ipc::v3::handlers::channel::{ping, goodbye, goodbye_ack};
use rekindle_transport_ipc::v3::handlers::stream::{open, payload, fin};
use rekindle_transport_ipc::v3::codec::channel::{ping as ping_codec, goodbye as goodbye_codec};
use rekindle_transport_ipc::v3::codec::stream::{open as open_codec, fin as fin_codec};
use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::session::state::SessionState;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

#[test]
fn full_session_lifecycle_handshake_stream_goodbye() {
    let (mut ctx, router) = make_test_context();
    assert_eq!(ctx.session_state(), SessionState::Established);

    // Ping/Pong
    let ping_payload = ping_codec::encode(&ping_codec::PingPayload {
        ping_nonce: 1, sender_epoch_ns: 0, last_seen_remote_seq: 0,
    });
    ping::handle(&mut ctx, &ping_payload).unwrap();
    let out = ctx.drain_outbound();
    assert_eq!(out.len(), 1); // PONG

    // Open stream
    let open_header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Open,
        stream_id: 1, header_flags: 0, chunk_index: 0, nonce: 0,
    };
    let chunk_data = b"hello from the other side";
    // Single-chunk Merkle: BLAKE3(BLAKE3(chunk_0))
    let content_hash = *blake3::Hasher::new()
        .update(blake3::hash(chunk_data).as_bytes())
        .finalize().as_bytes();
    let open_payload = open_codec::encode(&open_codec::StreamOpenPayload {
        transfer_id: uuid::Uuid::from_u128(1),
        expected_total_bytes: chunk_data.len() as u64,
        expected_chunk_count: 1,
        chunk_size: chunk_data.len() as u32,
        content_hash,
        lineage_kind: 0x01, dedup_hint: 0x03,
        clearance_required: Clearance::Public, conditions: vec![],
    });
    open::handle(&mut ctx, &open_header, &open_payload).unwrap();
    assert_eq!(ctx.stream_active_count(), 1);

    // Send one chunk
    let payload_header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Payload,
        stream_id: 1, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    payload::handle(&mut ctx, &payload_header, chunk_data).unwrap();

    // Verify chunk was staged in pending_bulk_deliveries
    assert_bulk_delivered(&mut ctx, 1, &[chunk_data.as_slice()]);

    // FIN
    let fin_header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Fin,
        stream_id: 1, header_flags: 0, chunk_index: 1, nonce: 2,
    };
    let fin_payload = fin_codec::encode(&fin_codec::StreamFinPayload {
        total_bytes: chunk_data.len() as u64,
        fault_count: 0,
        final_content_hash: content_hash,
        final_audit_link: ctx.inbound_chain().current_link(),
    });
    fin::handle(&mut ctx, &fin_header, &fin_payload).unwrap();

    // Should have STREAM_ACK in outbound
    let out = ctx.drain_outbound();
    assert!(out.iter().any(|f| matches!(f, OutboundFrame::Data { kind: StreamKind::Ack, .. })));

    // Verify on_bulk_complete fired on the router
    let completes = router.bulk_completes.lock();
    assert_eq!(completes.len(), 1, "FIN must fire on_bulk_complete");
    assert_eq!(completes[0].stream_id, 1);
    drop(completes);

    // Goodbye
    let goodbye_payload = goodbye_codec::encode(&goodbye_codec::GoodbyePayload {
        reason_code: 0, drain_timeout_ms: 30_000, final_session_seq: 3,
    });
    goodbye::handle(&mut ctx, &goodbye_payload).unwrap();
    assert_eq!(ctx.session_state(), SessionState::Draining);

    ctx.mark_local_goodbye_sent();
    let ack_payload = goodbye_codec::encode(&goodbye_codec::GoodbyePayload {
        reason_code: 0, drain_timeout_ms: 0, final_session_seq: 0,
    });
    ctx.drain_outbound();
    goodbye_ack::handle(&mut ctx, &ack_payload).unwrap();
    assert_eq!(ctx.session_state(), SessionState::Closed);
}
