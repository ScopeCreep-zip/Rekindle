use rekindle_transport_ipc::v3::context::PendingFinState;
use rekindle_transport_ipc::v3::dispatch::test_helpers::make_test_context;
use rekindle_transport_ipc::v3::stream::state::{StreamEvent, StreamState};

#[test]
fn stream_open_sets_state_to_open() {
    let (mut ctx, _router) = make_test_context();
    ctx.open_outbound_stream(0).unwrap();
    assert_eq!(ctx.outbound_stream_state(0), Some(StreamState::Open));
}

#[test]
fn fin_sent_transitions_open_to_closing() {
    let (mut ctx, _router) = make_test_context();
    ctx.open_outbound_stream(0).unwrap();
    ctx.transition_outbound_stream(0, StreamEvent::FinSent).unwrap();
    assert_eq!(ctx.outbound_stream_state(0), Some(StreamState::Closing));
}

#[test]
fn ack_for_fin_transitions_closing_to_closed() {
    let (mut ctx, _router) = make_test_context();
    ctx.open_outbound_stream(0).unwrap();
    ctx.transition_outbound_stream(0, StreamEvent::FinSent).unwrap();
    ctx.transition_outbound_stream(0, StreamEvent::AckForFinReceived).unwrap();
    assert_eq!(ctx.outbound_stream_state(0), Some(StreamState::Closed));
}

#[test]
fn ack_for_fin_on_open_stream_succeeds_directly_to_closed() {
    let (mut ctx, _router) = make_test_context();
    ctx.open_outbound_stream(0).unwrap();
    let result = ctx.transition_outbound_stream(0, StreamEvent::AckForFinReceived);
    assert!(result.is_ok(), "AckForFinReceived on Open must succeed — wire proves FIN was sent");
    assert_eq!(ctx.outbound_stream_state(0), Some(StreamState::Closed));
}

#[test]
fn closed_stream_id_reusable() {
    let (mut ctx, _router) = make_test_context();
    ctx.open_outbound_stream(0).unwrap();
    ctx.transition_outbound_stream(0, StreamEvent::FinSent).unwrap();
    ctx.transition_outbound_stream(0, StreamEvent::AckForFinReceived).unwrap();
    assert_eq!(ctx.outbound_stream_state(0), Some(StreamState::Closed));
    ctx.open_outbound_stream(0).unwrap();
    assert_eq!(ctx.outbound_stream_state(0), Some(StreamState::Open));
}

#[test]
fn pending_fin_counts_chunks() {
    let (mut ctx, _router) = make_test_context();
    ctx.open_outbound_stream(5).unwrap();

    let (tx, _rx) = crossbeam::channel::bounded(1);
    ctx.register_pending_fin(5, PendingFinState {
        chunk_count: 3,
        total_bytes: 1024,
        content_hash: [0xAA; 32],
        bulk_wire_tx: tx,
        last_chunk_seq: 42,
    });

    assert!(ctx.take_ready_pending_fin(5, 40).is_none());
    assert!(ctx.take_ready_pending_fin(5, 42).is_none());
    let completed = ctx.take_ready_pending_fin(5, 43);
    assert!(completed.is_some(), "reorder_next > last_chunk_seq must complete the PendingFin");
    let completed = completed.unwrap();
    assert_eq!(completed.chunk_count, 3);
    assert_eq!(completed.content_hash, [0xAA; 32]);
}

#[test]
fn pending_fin_removed_after_completion() {
    let (mut ctx, _router) = make_test_context();
    ctx.open_outbound_stream(7).unwrap();

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

#[test]
fn complete_sender_lifecycle() {
    let (mut ctx, _router) = make_test_context();

    ctx.open_outbound_stream(0).unwrap();
    ctx.create_reassembler(0);
    assert_eq!(ctx.outbound_stream_state(0), Some(StreamState::Open));

    let (tx, _rx) = crossbeam::channel::bounded(1);
    ctx.register_pending_fin(0, PendingFinState {
        chunk_count: 2,
        total_bytes: 2048,
        content_hash: [0xCC; 32],
        bulk_wire_tx: tx,
        last_chunk_seq: 5,
    });

    assert!(ctx.take_ready_pending_fin(0, 4).is_none());
    assert!(ctx.take_ready_pending_fin(0, 5).is_none());
    let completed = ctx.take_ready_pending_fin(0, 6);
    assert!(completed.is_some());

    ctx.transition_outbound_stream(0, StreamEvent::FinSent)
        .expect("FinSent must succeed on Open stream");
    assert_eq!(ctx.outbound_stream_state(0), Some(StreamState::Closing));

    ctx.transition_outbound_stream(0, StreamEvent::AckForFinReceived)
        .expect("AckForFinReceived must succeed on Closing stream");
    assert_eq!(ctx.outbound_stream_state(0), Some(StreamState::Closed));

    ctx.remove_reassembler(0);

    ctx.open_outbound_stream(0).unwrap();
    assert_eq!(ctx.outbound_stream_state(0), Some(StreamState::Open));
}
