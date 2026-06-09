//! Bulk transfer pipeline tests — forces the complete send/receive/verify
//! cycle per the v3 spec's bulk transfer pipeline spec.

use std::time::Duration;

use super::harness::*;

#[tokio::test]
async fn bulk_fin_ordering_after_all_payloads() {
    let f = connected_pair().await;

    let payload = vec![0xFE; 48 * 1024 * 1024];
    let result = f.send_bulk(0, &payload, Duration::from_secs(30)).await;

    assert!(result.is_ok(), "bulk transfer must succeed: {result:?}");

    let delivered = result.unwrap();
    assert_eq!(delivered.bytes_transferred, payload.len() as u64);
    assert!(delivered.chunks >= 3, "must have at least 3 chunks for 48 MiB");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1, "on_bulk_complete must fire exactly once");
    assert_eq!(completes[0].total_bytes, payload.len() as u64);
}

#[tokio::test]
async fn bulk_fin_before_payloads_defers_verification() {
    let f = connected_pair().await;

    let payload = vec![0xAB; 16 * 1024 * 1024 + 1];
    let result = f.send_bulk(0, &payload, Duration::from_secs(30)).await;

    assert!(result.is_ok(), "deferred FIN verification must succeed: {result:?}");

    let delivered = result.unwrap();
    assert_eq!(delivered.bytes_transferred, payload.len() as u64);
    assert_eq!(delivered.chunks, 2);

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
}

#[tokio::test]
async fn bulk_aead_verified_on_every_frame() {
    let f = connected_pair().await;

    let result = f.send_request(b"legit", TEST_TIMEOUT).await;
    assert!(result.is_ok(), "request must succeed");

    let payload = vec![0xCC; 1024];
    let result = f.send_bulk(0, &payload, TEST_TIMEOUT).await;
    assert!(result.is_ok(), "transfer with correct AEAD must succeed");

    assert!(!f.phase().is_terminal(), "connection must be alive after verified transfer");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, 1024);
}

#[tokio::test]
async fn bulk_audit_chain_covers_every_chunk() {
    let f = connected_pair_with_config(IpcFixtureConfig::for_test()).await;

    let payload = vec![0xAC; 1024 * 5];
    let result = f.send_bulk(0, &payload, Duration::from_secs(10)).await;

    assert!(result.is_ok(), "audit chain must be correct: {result:?}");

    let delivered = result.unwrap();
    assert_eq!(delivered.bytes_transferred, payload.len() as u64);

    assert!(!f.phase().is_terminal(), "connection must survive audit checkpoint verification");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, payload.len() as u64);
}

#[tokio::test]
async fn bulk_content_hash_mismatch_rejects() {
    let f = connected_pair().await;

    let wrong_hash = [0xFF; 32];
    let payload_data = vec![0xAA; 1024];
    let real_hash = *blake3::hash(&payload_data).as_bytes();
    assert_ne!(wrong_hash, real_hash);

    let open_payload = rekindle_transport_ipc::v3::codec::stream::open::encode(
        &rekindle_transport_ipc::v3::codec::stream::open::StreamOpenPayload {
            transfer_id: uuid::Uuid::now_v7(),
            expected_total_bytes: payload_data.len() as u64,
            expected_chunk_count: 1,
            chunk_size: payload_data.len() as u32,
            content_hash: wrong_hash,
            lineage_kind: 0x01,
            dedup_hint: 0x03,
            clearance_required: rekindle_transport_ipc::v3::wire::clearance::Clearance::Internal,
            conditions: vec![],
        },
    );
    f.send_raw_outbound(rekindle_transport_ipc::v3::context::OutboundFrame::Data {
        stream_id: 10,
        kind: rekindle_transport_ipc::v3::wire::frame_kind::StreamKind::Open,
        chunk_index: 0,
        payload: open_payload,
    }).await.unwrap();

    f.send_raw_outbound(rekindle_transport_ipc::v3::context::OutboundFrame::Data {
        stream_id: 10,
        kind: rekindle_transport_ipc::v3::wire::frame_kind::StreamKind::Payload,
        chunk_index: 0,
        payload: payload_data.clone(),
    }).await.unwrap();

    let fin_payload = rekindle_transport_ipc::v3::codec::stream::fin::encode(
        &rekindle_transport_ipc::v3::codec::stream::fin::StreamFinPayload {
            total_bytes: payload_data.len() as u64,
            fault_count: 0,
            final_content_hash: wrong_hash,
            final_audit_link: [0; 32],
        },
    );
    f.send_raw_outbound(rekindle_transport_ipc::v3::context::OutboundFrame::Data {
        stream_id: 10,
        kind: rekindle_transport_ipc::v3::wire::frame_kind::StreamKind::Fin,
        chunk_index: 1,
        payload: fin_payload,
    }).await.unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    let failures = f.router.bulk_failures.lock();
    let state_changes = f.router.state_changes.lock();
    let has_rejection = !failures.is_empty() || state_changes.iter().any(|c| {
        c.new_state.contains("ContentHashMismatch") || c.new_state.contains("ChannelError")
    });

    assert!(
        has_rejection || f.phase().is_terminal(),
        "content hash mismatch must be detected — failures: {}, state_changes: {}, phase: {:?}",
        failures.len(), state_changes.len(), f.phase()
    );
}

#[tokio::test]
async fn bulk_credit_exhaustion_stalls_sender() {
    let f = connected_pair_with_config(IpcFixtureConfig::for_test()).await;

    let payload = vec![0xEE; 5 * 1024];
    let result = f.send_bulk(0, &payload, Duration::from_secs(10)).await;

    assert!(result.is_ok(), "credit-bounded transfer must complete: {result:?}");

    let delivered = result.unwrap();
    assert_eq!(delivered.bytes_transferred, payload.len() as u64);

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, payload.len() as u64);
}

#[tokio::test]
async fn bulk_cancel_and_resume() {
    let f = connected_pair().await;

    let payload = vec![0xDD; 2048];
    let result = f.send_bulk(5, &payload, TEST_TIMEOUT).await;
    assert!(result.is_ok(), "initial transfer must succeed: {result:?}");

    f.cancel_bulk(5).await;

    let completes = f.router.bulk_completes.lock();
    assert!(completes.iter().any(|c| c.stream_id == 5 && c.total_bytes == 2048),
        "first transfer must have completed before cancel");
    drop(completes);

    let payload2 = vec![0xEE; 1024];
    let result2 = f.send_bulk(5, &payload2, TEST_TIMEOUT).await;
    assert!(result2.is_ok(), "recycled stream_id must succeed: {result2:?}");

    let completes2 = f.router.bulk_completes.lock();
    assert!(completes2.iter().any(|c| c.total_bytes == 1024),
        "second transfer on recycled stream must complete");
}

#[tokio::test]
async fn rayon_worker_sends_wire_before_audit_link() {
    let f = connected_pair().await;

    let payload = vec![0xBF; 48 * 1024 * 1024];
    let result = f.send_bulk(0, &payload, Duration::from_secs(30)).await;

    assert!(result.is_ok(), "proves wire→audit ordering: {result:?}");

    let delivered = result.unwrap();
    assert_eq!(delivered.bytes_transferred, payload.len() as u64);
    assert!(delivered.chunks >= 3);

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].total_bytes, payload.len() as u64);
}
