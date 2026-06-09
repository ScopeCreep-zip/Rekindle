use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::wire::frame_kind::AuditKind;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::audit::gap;
use rekindle_transport_ipc::v3::codec::audit::gap as gap_codec;
use rekindle_transport_ipc::v3::codec::audit::replay as replay_codec;

fn extract_replay_payload(out: &[OutboundFrame]) -> replay_codec::AuditReplayPayload {
    let frame = out.iter()
        .find(|f| matches!(f, OutboundFrame::Audit { kind: AuditKind::Replay, .. }))
        .expect("expected AUDIT_REPLAY (kind=0x05) in outbound");
    match frame {
        OutboundFrame::Audit { payload, .. } => {
            replay_codec::decode(payload).expect("AUDIT_REPLAY payload must decode")
        }
        _ => unreachable!(),
    }
}

#[test]
fn gap_with_retained_frames_produces_replay_with_correct_content() {
    let (mut ctx, router) = make_test_context();
    for seq in 0..10u64 {
        ctx.retention_mut().store(seq, vec![seq as u8; 100]);
    }

    let payload = gap_codec::encode(&gap_codec::AuditGapPayload {
        gap_id: uuid::Uuid::from_u128(1), gap_start_seq: 3, gap_end_seq: 5,
        gap_detected_ns: 0, expected_chain_link: [0; 32],
        missing_bitmap: vec![0b0000_0111],
    });
    gap::handle(&mut ctx, &payload).unwrap();

    let out = ctx.drain_outbound();
    let replay = extract_replay_payload(&out);

    assert_eq!(replay.gap_id, uuid::Uuid::from_u128(1));
    assert_eq!(replay.replay_frame_count, 3);
    assert_eq!(replay.replay_total_bytes, 300);
    assert_eq!(&replay.replayed_frames[0..100], &[3u8; 100]);
    assert_eq!(&replay.replayed_frames[100..200], &[4u8; 100]);
    assert_eq!(&replay.replayed_frames[200..300], &[5u8; 100]);
    assert_no_router_deliveries(&router);
}

#[test]
fn gap_partial_retention_replays_only_available_frames() {
    let (mut ctx, router) = make_test_context();
    ctx.retention_mut().store(3, vec![0x33; 50]);
    ctx.retention_mut().store(5, vec![0x55; 50]);

    let payload = gap_codec::encode(&gap_codec::AuditGapPayload {
        gap_id: uuid::Uuid::from_u128(2), gap_start_seq: 3, gap_end_seq: 5,
        gap_detected_ns: 0, expected_chain_link: [0; 32],
        missing_bitmap: vec![0b0000_0111],
    });
    gap::handle(&mut ctx, &payload).unwrap();

    let out = ctx.drain_outbound();
    let replay = extract_replay_payload(&out);

    assert_eq!(replay.replay_frame_count, 2);
    assert_eq!(replay.replay_total_bytes, 100);
    assert_eq!(&replay.replayed_frames[0..50], &[0x33; 50]);
    assert_eq!(&replay.replayed_frames[50..100], &[0x55; 50]);
    assert_no_router_deliveries(&router);
}

#[test]
fn gap_unfillable_when_retention_empty() {
    let (mut ctx, router) = make_test_context();

    let payload = gap_codec::encode(&gap_codec::AuditGapPayload {
        gap_id: uuid::Uuid::from_u128(3), gap_start_seq: 0, gap_end_seq: 5,
        gap_detected_ns: 0, expected_chain_link: [0; 32],
        missing_bitmap: vec![0b0011_1111],
    });
    gap::handle(&mut ctx, &payload).unwrap();

    let out = ctx.drain_outbound();
    let replay = extract_replay_payload(&out);

    assert_eq!(replay.gap_id, uuid::Uuid::from_u128(3));
    assert_eq!(replay.replay_frame_count, 0);
    assert_eq!(replay.replay_total_bytes, 0);
    assert!(replay.replayed_frames.is_empty());
    assert_no_router_deliveries(&router);
}

#[test]
fn gap_bitmap_selects_specific_frames() {
    let (mut ctx, router) = make_test_context();
    for seq in 10..20u64 {
        ctx.retention_mut().store(seq, vec![seq as u8; 32]);
    }

    let payload = gap_codec::encode(&gap_codec::AuditGapPayload {
        gap_id: uuid::Uuid::from_u128(4), gap_start_seq: 10, gap_end_seq: 17,
        gap_detected_ns: 0, expected_chain_link: [0; 32],
        missing_bitmap: vec![0b0010_0100],
    });
    gap::handle(&mut ctx, &payload).unwrap();

    let out = ctx.drain_outbound();
    let replay = extract_replay_payload(&out);

    assert_eq!(replay.replay_frame_count, 2);
    assert_eq!(replay.replay_total_bytes, 64);
    assert_eq!(&replay.replayed_frames[0..32], &[12u8; 32]);
    assert_eq!(&replay.replayed_frames[32..64], &[15u8; 32]);
    assert_no_router_deliveries(&router);
}
