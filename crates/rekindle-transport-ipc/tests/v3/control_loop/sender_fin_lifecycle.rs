//! Sender-side FIN lifecycle tests.
//!
//! These tests drive the control loop's outbound stream lifecycle
//! deterministically without sockets, rayon, or async I/O.
//! They verify the exact state transitions and frame emissions
//! that the socket E2E tests depend on.

use rekindle_transport_ipc::v3::context::PendingFinState;
use rekindle_transport_ipc::v3::dispatch::test_helpers::make_test_context;
use rekindle_transport_ipc::v3::stream::state::StreamState;

/// After STREAM_OPEN, the sender's stream is Open.
#[test]
fn stream_open_sets_state_to_open() {
    let (mut ctx, _router) = make_test_context();
    ctx.stream_registry_mut().open(0).unwrap();
    assert_eq!(ctx.stream_registry().state(0), Some(StreamState::Open));
}

/// After FinSent transition, the sender's stream is Closing.
#[test]
fn fin_sent_transitions_open_to_closing() {
    let (mut ctx, _router) = make_test_context();
    ctx.stream_registry_mut().open(0).unwrap();
    ctx.stream_registry_mut()
        .transition(0, rekindle_transport_ipc::v3::stream::state::StreamEvent::FinSent)
        .unwrap();
    assert_eq!(ctx.stream_registry().state(0), Some(StreamState::Closing));
}

/// After AckForFinReceived, the sender's stream is Closed.
#[test]
fn ack_for_fin_transitions_closing_to_closed() {
    let (mut ctx, _router) = make_test_context();
    ctx.stream_registry_mut().open(0).unwrap();
    ctx.stream_registry_mut()
        .transition(0, rekindle_transport_ipc::v3::stream::state::StreamEvent::FinSent)
        .unwrap();
    ctx.stream_registry_mut()
        .transition(0, rekindle_transport_ipc::v3::stream::state::StreamEvent::AckForFinReceived)
        .unwrap();
    assert_eq!(ctx.stream_registry().state(0), Some(StreamState::Closed));
}

/// AckForFinReceived on an Open stream (FinSent internal transition hasn't
/// fired yet) MUST succeed — the wire proves the FIN was sent because the
/// peer can only ACK what it received. The intermediate Closing state is
/// skipped. Open → Closed directly.
#[test]
fn ack_for_fin_on_open_stream_succeeds_directly_to_closed() {
    let (mut ctx, _router) = make_test_context();
    ctx.stream_registry_mut().open(0).unwrap();
    let result = ctx.stream_registry_mut()
        .transition(0, rekindle_transport_ipc::v3::stream::state::StreamEvent::AckForFinReceived);
    assert!(result.is_ok(), "AckForFinReceived on Open must succeed — wire proves FIN was sent");
    assert_eq!(ctx.stream_registry().state(0), Some(StreamState::Closed));
}

/// After Closed, the stream_id is reusable via open().
#[test]
fn closed_stream_id_reusable() {
    let (mut ctx, _router) = make_test_context();
    ctx.stream_registry_mut().open(0).unwrap();
    ctx.stream_registry_mut()
        .transition(0, rekindle_transport_ipc::v3::stream::state::StreamEvent::FinSent)
        .unwrap();
    ctx.stream_registry_mut()
        .transition(0, rekindle_transport_ipc::v3::stream::state::StreamEvent::AckForFinReceived)
        .unwrap();
    assert_eq!(ctx.stream_registry().state(0), Some(StreamState::Closed));
    ctx.stream_registry_mut().open(0).unwrap();
    assert_eq!(ctx.stream_registry().state(0), Some(StreamState::Open));
}

/// PendingFin readiness is determined by outbound reorder buffer position
/// relative to last_chunk_seq. FIN is ready when reorder_next > last_chunk_seq.
#[test]
fn pending_fin_counts_chunks() {
    let (mut ctx, _router) = make_test_context();
    ctx.stream_registry_mut().open(5).unwrap();

    let (tx, _rx) = crossbeam::channel::bounded(1);
    ctx.register_pending_fin(5, PendingFinState {
        chunk_count: 3,
        total_bytes: 1024,
        content_hash: [0xAA; 32],
        bulk_wire_tx: tx,
        last_chunk_seq: 42, // chunks used seqs 40, 41, 42
    });

    // Reorder buffer hasn't reached last_chunk_seq yet
    assert!(ctx.take_ready_pending_fin(5, 40).is_none());
    assert!(ctx.take_ready_pending_fin(5, 42).is_none());
    // Reorder buffer has advanced past last_chunk_seq
    let completed = ctx.take_ready_pending_fin(5, 43);
    assert!(completed.is_some(), "reorder_next > last_chunk_seq must complete the PendingFin");
    let completed = completed.unwrap();
    assert_eq!(completed.chunk_count, 3);
    assert_eq!(completed.content_hash, [0xAA; 32]);
}

/// After PendingFin completes, the stream_id is removed from pending_fins.
#[test]
fn pending_fin_removed_after_completion() {
    let (mut ctx, _router) = make_test_context();
    ctx.stream_registry_mut().open(7).unwrap();

    let (tx, _rx) = crossbeam::channel::bounded(1);
    ctx.register_pending_fin(7, PendingFinState {
        chunk_count: 1,
        total_bytes: 512,
        content_hash: [0xBB; 32],
        bulk_wire_tx: tx,
        last_chunk_seq: 10,
    });

    let _ = ctx.take_ready_pending_fin(7, 11);
    assert!(ctx.pending_fins_stream_ids().is_empty());
}

/// The complete sender lifecycle: open → register pending → seq advances →
/// FIN transition → ACK transition → closed → reusable.
#[test]
fn complete_sender_lifecycle() {
    let (mut ctx, _router) = make_test_context();

    // Step 1: STREAM_OPEN
    ctx.stream_registry_mut().open(0).unwrap();
    ctx.create_reassembler(0);
    assert_eq!(ctx.stream_registry().state(0), Some(StreamState::Open));

    // Step 2: Register PendingFin (BulkSender sends this after spawning rayon)
    let (tx, _rx) = crossbeam::channel::bounded(1);
    ctx.register_pending_fin(0, PendingFinState {
        chunk_count: 2,
        total_bytes: 2048,
        content_hash: [0xCC; 32],
        bulk_wire_tx: tx,
        last_chunk_seq: 5, // chunks used seqs 4 and 5
    });

    // Step 3: Reorder buffer advances as OutboundAuditLinks arrive
    assert!(ctx.take_ready_pending_fin(0, 4).is_none()); // not past last chunk
    assert!(ctx.take_ready_pending_fin(0, 5).is_none()); // not past (need >)
    let completed = ctx.take_ready_pending_fin(0, 6);     // past last chunk
    assert!(completed.is_some());

    // Step 4: emit_stream_fin transitions FinSent
    ctx.stream_registry_mut()
        .transition(0, rekindle_transport_ipc::v3::stream::state::StreamEvent::FinSent)
        .expect("FinSent must succeed on Open stream");
    assert_eq!(ctx.stream_registry().state(0), Some(StreamState::Closing));

    // Step 5: STREAM_ACK arrives → AckForFinReceived
    ctx.stream_registry_mut()
        .transition(0, rekindle_transport_ipc::v3::stream::state::StreamEvent::AckForFinReceived)
        .expect("AckForFinReceived must succeed on Closing stream");
    assert_eq!(ctx.stream_registry().state(0), Some(StreamState::Closed));

    // Step 6: Cleanup
    ctx.remove_reassembler(0);

    // Step 7: Reuse
    ctx.stream_registry_mut().open(0).unwrap();
    assert_eq!(ctx.stream_registry().state(0), Some(StreamState::Open));
}
