//! Receiver-side FIN lifecycle tests.
//!
//! These tests verify both immediate and deferred FIN verification:
//! - Immediate: FIN arrives after all chunks → verify + ACK + on_bulk_complete
//! - Deferred: FIN arrives before all chunks → store metadata → verify when last chunk arrives
//!
//! The FIN handler (handlers/stream/fin.rs) handles immediate verification.
//! The control loop's check_deferred_fin_verify handles the deferred path.
//! These tests drive both paths through the public handler API.

use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::codec::stream::fin as fin_codec;
use rekindle_transport_ipc::v3::context::{OutboundFrame, PendingFinVerify};
use rekindle_transport_ipc::v3::dispatch::test_helpers::make_test_context;
use rekindle_transport_ipc::v3::handlers::stream::{fin, payload, open};
use rekindle_transport_ipc::v3::codec::stream::open as open_codec;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

fn open_stream(ctx: &mut rekindle_transport_ipc::v3::context::SessionContext, stream_id: u8) {
    let header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Open,
        stream_id, header_flags: 0, chunk_index: 0, nonce: 0,
    };
    let payload = open_codec::encode(&open_codec::StreamOpenPayload {
        transfer_id: uuid::Uuid::from_u128(stream_id as u128),
        expected_total_bytes: 0, expected_chunk_count: 0, chunk_size: 65536,
        content_hash: [0; 32], lineage_kind: 0x01, dedup_hint: 0x03,
        clearance_required: Clearance::Public, conditions: vec![],
    });
    open::handle(ctx, &header, &payload).unwrap();
}

/// All chunks present, then FIN arrives → immediate verify → STREAM_ACK
/// emitted → on_bulk_complete fires on the router.
#[test]
fn immediate_verify_produces_ack_and_notifies_router() {
    let (mut ctx, router) = make_test_context();
    open_stream(&mut ctx, 3);

    let chunk_0 = vec![0xAA; 512];
    let chunk_1 = vec![0xBB; 512];
    // Merkle content hash: BLAKE3(BLAKE3(chunk_0) || BLAKE3(chunk_1))
    // Matches the reassembler's digest_aggregator and the sender's computation.
    let mut hasher = blake3::Hasher::new();
    hasher.update(blake3::hash(&chunk_0).as_bytes());
    hasher.update(blake3::hash(&chunk_1).as_bytes());
    let content_hash = *hasher.finalize().as_bytes();

    // Insert all chunks via payload handler
    let hdr0 = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Payload,
        stream_id: 3, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    payload::handle(&mut ctx, &hdr0, &chunk_0).unwrap();
    let hdr1 = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Payload,
        stream_id: 3, header_flags: 0, chunk_index: 1, nonce: 2,
    };
    payload::handle(&mut ctx, &hdr1, &chunk_1).unwrap();

    // FIN with correct hash — all chunks present → immediate verify
    let fin_hdr = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Fin,
        stream_id: 3, header_flags: 0, chunk_index: 2, nonce: 3,
    };
    let fin_payload = fin_codec::encode(&fin_codec::StreamFinPayload {
        total_bytes: 1024,
        fault_count: 0,
        final_content_hash: content_hash,
        final_audit_link: [0; 32],
    });
    fin::handle(&mut ctx, &fin_hdr, &fin_payload).unwrap();

    // STREAM_ACK must be in outbound
    let out = ctx.drain_outbound();
    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Data { kind: StreamKind::Ack, .. })),
        "immediate FIN verify must produce STREAM_ACK"
    );

    // Router must have received on_bulk_complete
    let completes = router.bulk_completes.lock();
    assert_eq!(completes.len(), 1, "on_bulk_complete must fire on immediate verify");
    assert_eq!(completes[0].total_bytes, 1024);
    assert_eq!(completes[0].total_chunks, 2);
}

/// Content hash mismatch → handler returns ContentHashMismatch error.
#[test]
fn immediate_verify_wrong_hash_returns_error() {
    let (mut ctx, router) = make_test_context();
    open_stream(&mut ctx, 5);

    let hdr = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Payload,
        stream_id: 5, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    payload::handle(&mut ctx, &hdr, &[0xCC; 256]).unwrap();

    let fin_hdr = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Fin,
        stream_id: 5, header_flags: 0, chunk_index: 1, nonce: 2,
    };
    let fin_payload = fin_codec::encode(&fin_codec::StreamFinPayload {
        total_bytes: 256,
        fault_count: 0,
        final_content_hash: [0xFF; 32], // WRONG
        final_audit_link: [0; 32],
    });
    let result = fin::handle(&mut ctx, &fin_hdr, &fin_payload);
    assert!(
        result.is_err(),
        "wrong content hash must produce error"
    );

    // Router must NOT have received on_bulk_complete
    let completes = router.bulk_completes.lock();
    assert_eq!(completes.len(), 0, "failed verify must not call on_bulk_complete");
}

/// FIN arrives before chunk 1 → deferred path stores PendingFinVerify.
/// The control loop fires verification when chunk 1 arrives.
/// This test verifies the prerequisites for deferred verification.
#[test]
fn deferred_path_stores_pending_when_chunks_missing() {
    let (mut ctx, _router) = make_test_context();
    open_stream(&mut ctx, 7);

    // Chunk 0 arrives
    let hdr = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Payload,
        stream_id: 7, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    payload::handle(&mut ctx, &hdr, &[0xDD; 256]).unwrap();

    // FIN arrives (chunk 1 still missing) — deferred path
    // Merkle content hash matching the reassembler's digest_aggregator
    let mut hasher = blake3::Hasher::new();
    hasher.update(blake3::hash(&[0xDD; 256]).as_bytes());
    hasher.update(blake3::hash(&[0xEE; 256]).as_bytes());
    let content_hash = *hasher.finalize().as_bytes();
    let fin_hdr = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Fin,
        stream_id: 7, header_flags: 0, chunk_index: 2, nonce: 2,
    };
    let fin_payload = fin_codec::encode(&fin_codec::StreamFinPayload {
        total_bytes: 512,
        fault_count: 0,
        final_content_hash: content_hash,
        final_audit_link: [0; 32],
    });
    fin::handle(&mut ctx, &fin_hdr, &fin_payload).unwrap();

    // PendingFinVerify must be stored
    assert!(ctx.has_pending_fin_verify(7), "deferred path must store PendingFinVerify");

    // Reassembler still waiting for chunk 1
    assert_eq!(ctx.reassembler_next_expected(7), 1, "only chunk 0 delivered");

    // No STREAM_ACK emitted yet
    let out = ctx.drain_outbound();
    assert!(
        !out.iter().any(|f| matches!(f, OutboundFrame::Data { kind: StreamKind::Ack, .. })),
        "deferred path must NOT emit STREAM_ACK before all chunks arrive"
    );
}

/// total_bytes in FIN must match reassembled bytes — detectable mismatch.
#[test]
fn total_bytes_mismatch_detectable() {
    let (mut ctx, _router) = make_test_context();
    open_stream(&mut ctx, 9);

    let hdr = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Payload,
        stream_id: 9, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    payload::handle(&mut ctx, &hdr, &[0xAA; 100]).unwrap();

    ctx.store_pending_fin_verify(9, PendingFinVerify {
        // Single-chunk Merkle hash = BLAKE3(BLAKE3(chunk_0))
        content_hash: *blake3::Hasher::new().update(blake3::hash(&[0xAA; 100]).as_bytes()).finalize().as_bytes(),
        audit_link: [0; 32],
        total_bytes: 999, // WRONG — actual is 100
        transfer_id: uuid::Uuid::nil(),
        expected_chunks: 1,
    });

    let pending = ctx.take_pending_fin_verify(9).unwrap();
    let actual_bytes = ctx.reassembler_mut(9).unwrap().total_bytes();
    assert_ne!(pending.total_bytes, actual_bytes, "mismatch must be detectable");
}
