use rekindle_transport_ipc::v3::context::{SessionContext, OutboundFrame};
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_bulk_delivered};
use rekindle_transport_ipc::v3::handlers::stream::{open, payload, fin};
use rekindle_transport_ipc::v3::handlers::HandlerError;
use rekindle_transport_ipc::v3::codec::stream::{
    open as open_codec, fin as fin_codec, ack as ack_codec,
};
use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

fn open_and_send_chunks(ctx: &mut SessionContext, stream_id: u8, chunks: &[&[u8]]) -> [u8; 32] {
    let total_bytes: u64 = chunks.iter().map(|c| c.len() as u64).sum();
    // Merkle content hash: BLAKE3(BLAKE3(chunk_0) || ... || BLAKE3(chunk_N))
    let mut hasher = blake3::Hasher::new();
    for chunk in chunks { hasher.update(blake3::hash(chunk).as_bytes()); }
    let content_hash = *hasher.finalize().as_bytes();

    let open_header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Open,
        stream_id, header_flags: 0, chunk_index: 0, nonce: 0,
    };
    let open_payload = open_codec::encode(&open_codec::StreamOpenPayload {
        transfer_id: uuid::Uuid::from_u128(u128::from(stream_id)),
        expected_total_bytes: total_bytes,
        expected_chunk_count: chunks.len() as u32,
        chunk_size: chunks[0].len() as u32,
        content_hash,
        lineage_kind: 0x01, dedup_hint: 0x03,
        clearance_required: Clearance::Public, conditions: vec![],
    });
    open::handle(ctx, &open_header, &open_payload).unwrap();

    for (i, chunk) in chunks.iter().enumerate() {
        let header = StreamHeaderInfo {
            frame_class: FrameClass::Stream, frame_kind: StreamKind::Payload,
            stream_id, header_flags: 0, chunk_index: i as u32, nonce: i as u64 + 1,
        };
        payload::handle(ctx, &header, chunk).unwrap();
    }
    content_hash
}

fn extract_stream_ack(out: &[OutboundFrame]) -> ack_codec::StreamAckPayload {
    let frame = out.iter()
        .find(|f| matches!(f, OutboundFrame::Data { kind: StreamKind::Ack, .. }))
        .expect("expected STREAM_ACK in outbound");
    match frame {
        OutboundFrame::Data { payload, .. } => {
            ack_codec::decode(payload).expect("STREAM_ACK payload must decode")
        }
        _ => unreachable!(),
    }
}

#[test]
fn fin_with_correct_hash_produces_ack_with_matching_transfer_id() {
    let (mut ctx, router) = make_test_context();
    let chunks: Vec<&[u8]> = vec![b"hello", b"world"];
    let content_hash = open_and_send_chunks(&mut ctx, 5, &chunks);
    // Drain bulk deliveries from payload handling
    assert_bulk_delivered(&mut ctx, 5, &[b"hello", b"world"]);
    ctx.drain_outbound();

    let fin_header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Fin,
        stream_id: 5, header_flags: 0, chunk_index: 2, nonce: 3,
    };
    let fin_payload = fin_codec::encode(&fin_codec::StreamFinPayload {
        total_bytes: 10,
        fault_count: 0,
        final_content_hash: content_hash,
        final_audit_link: ctx.inbound_chain().current_link(),
    });
    fin::handle(&mut ctx, &fin_header, &fin_payload).unwrap();

    let out = ctx.drain_outbound();
    let ack = extract_stream_ack(&out);
    assert_eq!(ack.transfer_id, uuid::Uuid::nil());
    assert_eq!(ack.ack_byte_count, 10);
    assert_eq!(ack.ack_chunk_count, 2);

    // on_bulk_complete must have fired on the router
    let completes = router.bulk_completes.lock();
    assert_eq!(completes.len(), 1, "FIN with correct hash must fire on_bulk_complete");
    assert_eq!(completes[0].stream_id, 5);
    assert_eq!(completes[0].total_bytes, 10);
    assert_eq!(completes[0].total_chunks, 2);
}

#[test]
fn fin_with_correct_hash_verifies_assembled_payload() {
    let (mut ctx, router) = make_test_context();
    let chunk_a = b"AAAA";
    let chunk_b = b"BBBB";
    let mut h = blake3::Hasher::new();
    h.update(blake3::hash(chunk_a).as_bytes());
    h.update(blake3::hash(chunk_b).as_bytes());
    let expected_hash = *h.finalize().as_bytes();
    let content_hash = open_and_send_chunks(&mut ctx, 7, &[chunk_a, chunk_b]);
    assert_eq!(content_hash, expected_hash);
    assert_bulk_delivered(&mut ctx, 7, &[b"AAAA", b"BBBB"]);
    ctx.drain_outbound();

    let fin_header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Fin,
        stream_id: 7, header_flags: 0, chunk_index: 2, nonce: 3,
    };
    let fin_payload = fin_codec::encode(&fin_codec::StreamFinPayload {
        total_bytes: 8,
        fault_count: 0,
        final_content_hash: content_hash,
        final_audit_link: ctx.inbound_chain().current_link(),
    });
    fin::handle(&mut ctx, &fin_header, &fin_payload).unwrap();

    let out = ctx.drain_outbound();
    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Data { kind: StreamKind::Ack, .. })),
        "verified content hash must produce STREAM_ACK"
    );

    let completes = router.bulk_completes.lock();
    assert_eq!(completes.len(), 1, "FIN verify must fire on_bulk_complete");
    assert_eq!(completes[0].total_bytes, 8);
    assert_eq!(completes[0].total_chunks, 2);
}

#[test]
fn fin_with_wrong_hash_produces_content_hash_mismatch_error() {
    let (mut ctx, router) = make_test_context();
    let chunks: Vec<&[u8]> = vec![b"hello", b"world"];
    open_and_send_chunks(&mut ctx, 5, &chunks);
    // Drain bulk deliveries — chunks were delivered during payload handling
    assert_bulk_delivered(&mut ctx, 5, &[b"hello", b"world"]);
    ctx.drain_outbound();

    let fin_header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Fin,
        stream_id: 5, header_flags: 0, chunk_index: 2, nonce: 3,
    };
    let fin_payload = fin_codec::encode(&fin_codec::StreamFinPayload {
        total_bytes: 10,
        fault_count: 0,
        final_content_hash: [0xFF; 32],
        final_audit_link: [0; 32],
    });
    let result = fin::handle(&mut ctx, &fin_header, &fin_payload);
    match result {
        Err(HandlerError::ContentHashMismatch) => {}
        Err(other) => panic!("expected ContentHashMismatch, got {other:?}"),
        Ok(()) => panic!("fin with wrong hash must fail"),
    }

    // on_bulk_complete must NOT have fired
    let completes = router.bulk_completes.lock();
    assert_eq!(completes.len(), 0, "failed FIN must not fire on_bulk_complete");
}
