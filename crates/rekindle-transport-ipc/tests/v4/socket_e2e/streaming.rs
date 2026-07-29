//! Streaming transport tests — SharedArena over real sockets.
//! Proves server→client arena streaming works end-to-end through the
//! Handoff lane, AEAD encryption, ArenaWrite/SlotRelease handler cycle.
//!
//! These tests exercise the SpiritStream 4K120fps path:
//! StreamingSender::send_frame → SharedArena CAS → ArenaWrite frame →
//! socket → peer Handoff lane → arena_write handler → router.on_arena_write →
//! SlotRelease → return_slot CAS.

use std::time::Duration;

use super::harness::*;

/// Server sends a single frame via SharedArena to the client.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_to_client_single_arena_frame() {
    init_tracing();
    let f = connected_pair().await;

    // Verify arena was negotiated
    let sender = f.streaming_sender();
    if sender.is_none() {
        eprintln!("SHARED_ARENA not negotiated — skipping streaming test");
        return;
    }
    let sender = sender.unwrap();
    assert!(sender.has_arena(), "StreamingSender must have arena");

    // Send a frame larger than inline threshold (64 KiB) to force SharedMem tier
    let frame_data = payload(128 * 1024);
    let result = sender.send_frame(&frame_data).await;
    assert!(result.is_ok(), "send_frame must succeed: {result:?}");

    let result = result.unwrap();
    assert!(
        matches!(result, rekindle_transport_ipc::v4::streaming::send::StreamingSendResult::SentViaArena(_)),
        "128 KiB frame must use SharedMem tier, got {result:?}"
    );

    // Wait for the frame to traverse the socket and be processed
    tokio::time::sleep(Duration::from_millis(200)).await;

    let arena_writes = f.router.arena_writes.lock();
    assert_eq!(arena_writes.len(), 1, "on_arena_write must fire exactly once");
    assert_eq!(arena_writes[0].data.len(), frame_data.len(), "data length must match");
    assert_eq!(arena_writes[0].data, frame_data, "data content must match");
}

/// Server sends multiple frames simulating a burst.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_to_client_burst_120_frames() {
    init_tracing();
    let f = connected_pair().await;

    let sender = match f.streaming_sender() {
        Some(s) => s,
        None => { eprintln!("SHARED_ARENA not negotiated — skipping"); return; }
    };

    let frame_size = 128 * 1024; // 128 KiB per frame
    let frame_data = payload(frame_size);
    let mut sent = 0u32;

    for _ in 0..120 {
        match sender.send_frame(&frame_data).await {
            Ok(rekindle_transport_ipc::v4::streaming::send::StreamingSendResult::SentViaArena(_)) => {
                sent += 1;
            }
            Ok(rekindle_transport_ipc::v4::streaming::send::StreamingSendResult::UseInline) => {
                panic!("128 KiB frame must not use inline tier");
            }
            Err(rekindle_transport_ipc::v4::streaming::send::StreamingSendError::AllSlotsBusy) => {
                // Backpressure — acceptable, try again after a short wait
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Err(e) => panic!("send_frame failed: {e}"),
            _ => {}
        }
    }

    // Wait for all frames to be processed
    tokio::time::sleep(Duration::from_millis(500)).await;

    let arena_writes = f.router.arena_writes.lock();
    assert!(
        arena_writes.len() >= sent as usize / 2,
        "at least half of {sent} sent frames must be delivered, got {}",
        arena_writes.len()
    );

    // Verify data integrity on delivered frames
    for aw in arena_writes.iter() {
        assert_eq!(aw.data.len(), frame_size, "frame size must match");
        assert_eq!(aw.data, frame_data, "frame data must match");
    }
}

/// Small payload uses inline tier (falls back to BulkSender/datagram).
#[cfg(target_os = "linux")]
#[tokio::test]
async fn small_payload_uses_inline_tier() {
    let f = connected_pair().await;

    let sender = match f.streaming_sender() {
        Some(s) => s,
        None => { eprintln!("SHARED_ARENA not negotiated — skipping"); return; }
    };

    let small_data = vec![0xAA; 1024]; // 1 KiB — below 64 KiB threshold
    let result = sender.send_frame(&small_data).await
        .expect("send_frame must succeed");

    assert!(
        matches!(result, rekindle_transport_ipc::v4::streaming::send::StreamingSendResult::UseInline),
        "1 KiB frame must use inline tier"
    );
}

/// Streaming concurrent with control plane — rotation during streaming.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_survives_key_rotation() {
    init_tracing();
    let f = connected_pair().await;

    let sender = match f.streaming_sender() {
        Some(s) => s,
        None => { eprintln!("SHARED_ARENA not negotiated — skipping"); return; }
    };

    let frame_data = payload(128 * 1024);

    // Send some frames before rotation
    for _ in 0..5 {
        let _ = sender.send_frame(&frame_data).await;
    }

    // Rotate keys while streaming is active
    f.rotate_keys(TEST_TIMEOUT).await
        .expect("rotation during streaming must succeed");

    // Send more frames after rotation — new epoch keys
    for _ in 0..5 {
        let _ = sender.send_frame(&frame_data).await;
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Connection must still be alive
    f.send_request(b"alive-after-streaming-rotation", TEST_TIMEOUT).await
        .expect("control plane must work after streaming + rotation");

    let arena_writes = f.router.arena_writes.lock();
    assert!(
        !arena_writes.is_empty(),
        "at least some arena frames must be delivered across rotation"
    );
}

/// Streaming concurrent with bulk transfer — both paths active simultaneously.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_concurrent_with_bulk() {
    init_tracing();
    let f = connected_pair().await;

    let sender = match f.streaming_sender() {
        Some(s) => s,
        None => { eprintln!("SHARED_ARENA not negotiated — skipping"); return; }
    };

    let frame_data = payload(128 * 1024);
    let bulk_data = vec![0xDD; 2 * 1024 * 1024];

    // Launch streaming and bulk concurrently
    let streaming_handle = {
        let sender = sender.clone();
        let frame_data = frame_data.clone();
        tokio::spawn(async move {
            let mut sent = 0u32;
            for _ in 0..30 {
                match sender.send_frame(&frame_data).await {
                    Ok(_) => sent += 1,
                    Err(rekindle_transport_ipc::v4::streaming::send::StreamingSendError::AllSlotsBusy) => {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    Err(e) => panic!("streaming send failed: {e}"),
                }
            }
            sent
        })
    };

    let bulk_result = f.send_bulk(0, &bulk_data, Duration::from_secs(15)).await;

    let streaming_sent = streaming_handle.await.expect("streaming task must not panic");

    assert!(bulk_result.is_ok(), "bulk must succeed concurrent with streaming: {bulk_result:?}");
    assert!(streaming_sent > 0, "at least some streaming frames must send");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 1, "bulk transfer must complete");

    let arena_writes = f.router.arena_writes.lock();
    assert!(
        !arena_writes.is_empty(),
        "streaming frames must be delivered concurrent with bulk"
    );
}
