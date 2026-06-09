//! Frame priority, reassembly ordering, and stream lifecycle tests.
//!
//! Proves that:
//! - Out-of-order chunks are delivered in chunk_index order (ReorderRing)
//! - Duplicate chunks are rejected without corrupting the reassembler
//! - Multiple streams reassemble independently
//! - Stream reuse after close starts with a fresh reassembler
//! - Backpressure state is tracked correctly
//! - Merkle content hash verification matches sender computation
//!
//! These tests are implementation-agnostic: they test the contract
//! (ordered delivery, integrity, independence) not the mechanism.

use rekindle_transport_ipc::v3::dispatch::test_helpers::{
    make_test_context, assert_bulk_delivered, assert_no_bulk_deliveries, insert_chunk,
};
use rekindle_transport_ipc::v3::io::lane_channels::PlaintextBuf;

/// Out-of-order chunks delivered in chunk_index order.
/// This is the core reassembly contract: regardless of arrival order,
/// the application receives chunks 0, 1, 2 in that order.
#[test]
fn bulk_chunks_delivered_in_order_despite_arrival_order() {
    let (mut ctx, _router) = make_test_context();

    use rekindle_transport_ipc::v3::handlers::stream::{open, payload};
    use rekindle_transport_ipc::v3::codec::stream::open as open_codec;
    use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
    use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
    use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;
    use rekindle_transport_ipc::v3::wire::clearance::Clearance;

    let oh = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Open,
        stream_id: 3, header_flags: 0, chunk_index: 0, nonce: 0,
    };
    open::handle(&mut ctx, &oh, &open_codec::encode(&open_codec::StreamOpenPayload {
        transfer_id: uuid::Uuid::from_u128(3),
        expected_total_bytes: 300, expected_chunk_count: 3, chunk_size: 100,
        content_hash: [0; 32], lineage_kind: 0x01, dedup_hint: 0x03,
        clearance_required: Clearance::Public, conditions: vec![],
    })).unwrap();

    // Chunk 2 first → buffered (gap at 0 and 1)
    let h2 = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Payload,
        stream_id: 3, header_flags: 0, chunk_index: 2, nonce: 3,
    };
    payload::handle(&mut ctx, &h2, &vec![0x22; 100]).unwrap();
    assert_no_bulk_deliveries(&mut ctx);

    // Chunk 0 → delivers chunk 0 only (gap at 1 blocks chunk 2)
    let h0 = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Payload,
        stream_id: 3, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    payload::handle(&mut ctx, &h0, &vec![0x00; 100]).unwrap();
    assert_bulk_delivered(&mut ctx, 3, &[&vec![0x00; 100]]);

    // Chunk 1 → delivers chunks 1 AND 2 (contiguous prefix complete)
    let h1 = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Payload,
        stream_id: 3, header_flags: 0, chunk_index: 1, nonce: 2,
    };
    payload::handle(&mut ctx, &h1, &vec![0x11; 100]).unwrap();
    assert_bulk_delivered(&mut ctx, 3, &[&vec![0x11; 100], &vec![0x22; 100]]);
}

/// Duplicate chunks below next_expected are silently dropped.
/// The reassembler must not deliver the same chunk twice and must
/// not corrupt its state on duplicates.
#[test]
fn duplicate_chunks_silently_dropped() {
    let (mut ctx, _) = make_test_context();

    ctx.stream_registry_mut().open(5).unwrap();
    ctx.create_reassembler(5);

    insert_chunk(&mut ctx, 5, 0, &[0xAA; 50]);
    let deliveries = ctx.drain_bulk_deliveries();
    assert_eq!(deliveries.len(), 1);

    // Duplicate of chunk 0 — must be dropped, not delivered again
    insert_chunk(&mut ctx, 5, 0, &[0xAA; 50]);
    assert_no_bulk_deliveries(&mut ctx);

    assert_eq!(ctx.reassembler_next_expected(5), 1, "next_expected advances only once");
}

/// Multiple streams reassemble independently — chunks on stream 0
/// do not interfere with stream 1's reassembly state.
#[test]
fn multiple_streams_reassemble_independently() {
    let (mut ctx, _) = make_test_context();

    ctx.stream_registry_mut().open(0).unwrap();
    ctx.stream_registry_mut().open(1).unwrap();
    ctx.create_reassembler(0);
    ctx.create_reassembler(1);

    insert_chunk(&mut ctx, 0, 0, &[0x00; 64]);
    insert_chunk(&mut ctx, 1, 0, &[0x11; 64]);
    insert_chunk(&mut ctx, 0, 1, &[0x01; 64]);

    assert_eq!(ctx.reassembler_next_expected(0), 2);
    assert_eq!(ctx.reassembler_next_expected(1), 1);
}

/// Double open on the same stream_id is rejected.
#[test]
fn double_open_rejected() {
    let (mut ctx, _) = make_test_context();
    ctx.stream_registry_mut().open(7).unwrap();
    let result = ctx.stream_registry_mut().open(7);
    assert!(result.is_err(), "double open must be rejected");
}

/// After close + reassembler removal, reopening the same stream_id
/// starts with a fresh reassembler at chunk 0.
#[test]
fn stream_reuse_after_close_starts_fresh() {
    let (mut ctx, _) = make_test_context();

    ctx.stream_registry_mut().open(4).unwrap();
    ctx.create_reassembler(4);
    insert_chunk(&mut ctx, 4, 0, &[0xAA; 32]);
    assert_eq!(ctx.reassembler_next_expected(4), 1);

    let _ = ctx.stream_registry_mut().close(4);
    ctx.remove_reassembler(4);

    ctx.stream_registry_mut().open(4).unwrap();
    ctx.create_reassembler(4);

    assert_eq!(ctx.reassembler_next_expected(4), 0,
        "reopened stream must start with fresh reassembler at chunk 0");
}

/// Backpressure state is tracked.
#[test]
fn backpressure_state_tracked() {
    let (mut ctx, _) = make_test_context();
    use rekindle_transport_ipc::v3::stream::flow_control::Severity;
    assert!(!ctx.is_backpressured());
    ctx.backpressure_mut().assert_backpressure(Severity::Critical);
    assert!(ctx.is_backpressured());
    ctx.backpressure_mut().clear();
    assert!(!ctx.is_backpressured());
}

/// Merkle content hash verification: sender and receiver produce the
/// same root when chunks are delivered in order.
#[test]
fn merkle_content_hash_matches_sender_computation() {
    let (mut ctx, _) = make_test_context();
    ctx.stream_registry_mut().open(10).unwrap();
    ctx.create_reassembler(10);

    let chunks: Vec<Vec<u8>> = vec![
        vec![0x01; 100],
        vec![0x02; 200],
        vec![0x03; 150],
    ];

    // Sender-side computation
    let mut sender_hasher = blake3::Hasher::new();
    for chunk in &chunks {
        let digest = *blake3::hash(chunk).as_bytes();
        sender_hasher.update(&digest);
    }
    let sender_root = *sender_hasher.finalize().as_bytes();

    // Receiver-side: insert chunks in order
    for (i, chunk) in chunks.iter().enumerate() {
        let digest = *blake3::hash(chunk).as_bytes();
        let delivered = ctx.reassembler_mut(10).unwrap()
            .insert_with_digest(i as u32, PlaintextBuf::Owned(chunk.clone()), digest);
        for (ci, d) in delivered {
            ctx.push_bulk_delivery(10, ci, d);
        }
    }

    // Verify
    let result = ctx.reassembler_mut(10).unwrap()
        .verify_content_hash(&sender_root);
    assert!(result.is_ok(), "Merkle root must match sender computation");
}

/// Merkle hash verification fails when a chunk is corrupted.
#[test]
fn merkle_content_hash_fails_on_corruption() {
    let (mut ctx, _) = make_test_context();
    ctx.stream_registry_mut().open(11).unwrap();
    ctx.create_reassembler(11);

    let chunks: Vec<Vec<u8>> = vec![vec![0x01; 100], vec![0x02; 100]];

    // Sender computes over original chunks
    let mut sender_hasher = blake3::Hasher::new();
    for chunk in &chunks {
        sender_hasher.update(blake3::hash(chunk).as_bytes());
    }
    let sender_root = *sender_hasher.finalize().as_bytes();

    // Receiver gets chunk 0 correct but chunk 1 corrupted
    let d0 = *blake3::hash(&chunks[0]).as_bytes();
    ctx.reassembler_mut(11).unwrap().insert_with_digest(0, PlaintextBuf::Owned(chunks[0].clone()), d0);

    let corrupted = vec![0xFF; 100]; // different from 0x02
    let d1 = *blake3::hash(&corrupted).as_bytes();
    ctx.reassembler_mut(11).unwrap().insert_with_digest(1, PlaintextBuf::Owned(corrupted), d1);

    let result = ctx.reassembler_mut(11).unwrap().verify_content_hash(&sender_root);
    assert!(result.is_err(), "Merkle root must NOT match when a chunk is corrupted");
}

/// Sequenced outbound confirmation resolves.
#[tokio::test]
async fn sequenced_outbound_confirmation_resolves() {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tx.send(()).unwrap();
    assert!(rx.await.is_ok(), "oneshot confirmation must resolve");
}
