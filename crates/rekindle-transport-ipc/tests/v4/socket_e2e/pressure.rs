//! Pressure tests — sustained concurrent workloads that simulate
//! real application patterns: SpiritStream + Rekindle + OpenSesame
//! running simultaneously on the same transport.

use std::time::Duration;

use super::harness::*;

/// Sustained control plane under bulk transfer load.
/// Simulates Rekindle file sync + daemon RPC simultaneously.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn control_plane_under_bulk_pressure() {
    let f = connected_pair().await;

    let bulk_data = vec![0xEE; 4 * 1024 * 1024];

    // Launch bulk transfer in background
    let bulk_handle = {
        let bulk_sender = f.bulk_sender();
        let clearance = f.agreed_clearance();
        let data = bulk_data.clone();
        tokio::spawn(async move {
            for i in 0..5u8 {
                bulk_sender.send(i, &data, clearance).await
                    .unwrap_or_else(|e| panic!("bulk {i} failed: {e:?}"));
            }
        })
    };

    // Concurrently send control plane requests
    for i in 0..100u32 {
        f.send_request(&i.to_le_bytes(), TEST_TIMEOUT).await
            .unwrap_or_else(|e| panic!("request {i} failed during bulk pressure: {e:?}"));
    }

    bulk_handle.await.expect("bulk task must not panic");

    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 100, "all 100 control requests must arrive under bulk pressure");
}

/// Bulk + rotation interleaved — key rotation during active transfer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rotation_during_bulk_transfer() {
    init_tracing();
    let f = connected_pair().await;

    // Start a bulk transfer
    let payload = vec![0xFF; 2 * 1024 * 1024];
    let bulk_handle = tokio::spawn({
        let f_send_bulk = f.bulk_sender();
        let clearance = f.agreed_clearance();
        let data = payload.clone();
        async move {
            f_send_bulk.send(0, &data, clearance).await
                .expect("bulk must succeed across rotation");
        }
    });

    // Rotate keys while bulk is in flight
    tokio::time::sleep(Duration::from_millis(10)).await;
    f.rotate_keys(Duration::from_secs(10)).await
        .expect("rotation during bulk must succeed");

    bulk_handle.await.expect("bulk task must complete");

    // Verify connection is still functional
    f.send_request(b"post-rotation-bulk-alive", TEST_TIMEOUT).await
        .expect("control plane must work after rotation + bulk");
}

/// Multiple clients sending bulk transfers simultaneously.
/// Simulates multi-device workspace sync.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multi_client_concurrent_bulk() {
    let f = connected_pair().await;

    // Connect additional clients
    let client2 = f.connect_additional_client().await;
    let client3 = f.connect_additional_client().await;

    let payload = vec![0xAA; 512 * 1024];

    let (r1, r2, r3) = tokio::join!(
        f.send_bulk(0, &payload, Duration::from_secs(15)),
        client2.send_bulk(0, &payload, Duration::from_secs(15)),
        client3.send_bulk(0, &payload, Duration::from_secs(15)),
    );

    assert!(r1.is_ok(), "client 1 bulk must succeed: {r1:?}");
    assert!(r2.is_ok(), "client 2 bulk must succeed: {r2:?}");
    assert!(r3.is_ok(), "client 3 bulk must succeed: {r3:?}");

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 3, "all 3 client bulk transfers must complete");

    client2.shutdown().await;
    client3.shutdown().await;
}

/// Sustained mixed workload — the full SpiritStream + Rekindle scenario.
/// Streaming frames + bulk transfers + control plane + key rotation.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_mixed_workload() {
    init_tracing();
    let f = connected_pair().await;

    let sender = match f.streaming_sender() {
        Some(s) => s,
        None => { eprintln!("SHARED_ARENA not negotiated — skipping"); return; }
    };

    let frame_data = payload(128 * 1024);
    let bulk_data = vec![0xBB; 1024 * 1024];

    // Streaming task — 60 frames (simulates ~0.5 sec of 120fps)
    let streaming = {
        let sender = sender.clone();
        let data = frame_data.clone();
        tokio::spawn(async move {
            let mut sent = 0u32;
            for _ in 0..60 {
                match sender.send_frame(&data).await {
                    Ok(_) => sent += 1,
                    Err(rekindle_transport_ipc::v4::streaming::send::StreamingSendError::AllSlotsBusy) => {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    Err(e) => panic!("streaming failed: {e}"),
                }
            }
            sent
        })
    };

    // Bulk task — 2 transfers
    let bulk = tokio::spawn({
        let bulk_sender = f.bulk_sender();
        let clearance = f.agreed_clearance();
        let data = bulk_data.clone();
        async move {
            for i in 0..2u8 {
                bulk_sender.send(i, &data, clearance).await
                    .unwrap_or_else(|e| panic!("bulk {i} failed: {e:?}"));
            }
        }
    });

    // Control plane — 50 requests
    for i in 0..50u32 {
        f.send_request(&i.to_le_bytes(), TEST_TIMEOUT).await
            .unwrap_or_else(|e| panic!("request {i} failed: {e:?}"));
    }

    // Key rotation mid-workload
    f.rotate_keys(Duration::from_secs(10)).await
        .expect("rotation during mixed workload must succeed");

    // More control plane after rotation
    for i in 50..100u32 {
        f.send_request(&i.to_le_bytes(), TEST_TIMEOUT).await
            .unwrap_or_else(|e| panic!("post-rotation request {i} failed: {e:?}"));
    }

    let streaming_sent = streaming.await.expect("streaming task must complete");
    bulk.await.expect("bulk task must complete");

    // Verify all paths delivered
    let requests = f.router.requests.lock();
    assert_eq!(requests.len(), 100, "all 100 requests must arrive under mixed load");
    drop(requests);

    let completes = f.router.bulk_completes.lock();
    assert_eq!(completes.len(), 2, "both bulk transfers must complete");
    drop(completes);

    let arena_writes = f.router.arena_writes.lock();
    assert!(
        !arena_writes.is_empty(),
        "streaming frames must deliver under mixed load (sent {streaming_sent})"
    );

    // Connection must be alive after the full workload
    assert!(
        !f.phase().is_terminal(),
        "connection must survive full mixed workload"
    );
}
