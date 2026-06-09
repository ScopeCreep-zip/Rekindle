//! Sustained-pressure integration tests.
//!
//! Proves the IPC transport operates correctly under sustained max-throughput
//! load without: blocking the host, leaking memory, dropping below throughput
//! floor, or starving non-bulk work (heartbeats, control frames).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rekindle_transport_ipc::v3::bulk::counters as diag;

use super::harness::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sustained_bulk_no_leak_no_stall() {
    diag::reset_all_diagnostics();

    let config = IpcFixtureConfig {
        retention_max_bytes: Some(4 * 1024 * 1024),
        retention_max_frames: Some(64),
        ..IpcFixtureConfig::for_test()
    };
    let f = connected_pair_with_config(config).await;
    let data = payload(1024 * 1024);
    let transfers = 50;

    for _ in 0..5 {
        f.send_bulk(0, &data, Duration::from_secs(30)).await
            .expect("warmup transfer failed");
    }
    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    unsafe { libc::malloc_trim(0); }
    let rss_before = get_rss_bytes();
    let start = Instant::now();

    for i in 0..transfers {
        f.send_bulk(0, &data, Duration::from_secs(30)).await
            .unwrap_or_else(|e| panic!("transfer {i} failed: {e}"));

        if i % 5 == 0 || i == transfers - 1 {
            #[cfg(target_os = "linux")]
            #[allow(unsafe_code)]
            unsafe { libc::malloc_trim(0); }
            let rss_now = get_rss_bytes();
            let growth = rss_now.saturating_sub(rss_before);
            eprintln!(
                "transfer {i:>3}: RSS {} MiB  growth {} MiB  delta/transfer {} KiB",
                rss_now / (1024 * 1024),
                growth / (1024 * 1024),
                if i > 0 { growth / (i as usize) / 1024 } else { 0 },
            );
        }
    }

    let elapsed = start.elapsed();
    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    unsafe { libc::malloc_trim(0); }
    let rss_after = get_rss_bytes();
    let rss_growth = rss_after.saturating_sub(rss_before);

    eprintln!("=== PIPELINE ALLOCATION DIAGNOSTICS (sustained) ===");
    eprintln!("send_plaintext_alloc:   {} MiB", diag::DIAG_SEND_PLAINTEXT_BYTES.load(Ordering::Relaxed) / (1024 * 1024));
    eprintln!("wire_pool hits/miss/ret/overflow: {}/{}/{}/{}",
        diag::DIAG_WIRE_POOL_HITS.load(Ordering::Relaxed),
        diag::DIAG_WIRE_POOL_MISSES.load(Ordering::Relaxed),
        diag::DIAG_WIRE_POOL_RETURNS.load(Ordering::Relaxed),
        diag::DIAG_WIRE_POOL_OVERFLOW_DROPS.load(Ordering::Relaxed));
    eprintln!("recv_plaintext_alloc:   {} MiB", diag::DIAG_RECV_PLAINTEXT_BYTES.load(Ordering::Relaxed) / (1024 * 1024));
    eprintln!("retention_stored:       {} MiB ({} frames)",
        diag::DIAG_RETENTION_STORED_BYTES.load(Ordering::Relaxed) / (1024 * 1024),
        diag::DIAG_RETENTION_STORED_FRAMES.load(Ordering::Relaxed));
    eprintln!("recv_delivered:         {} MiB ({} frames)",
        diag::DIAG_RECV_DELIVERED_BYTES.load(Ordering::Relaxed) / (1024 * 1024),
        diag::DIAG_RECV_DELIVERED_FRAMES.load(Ordering::Relaxed));
    eprintln!("RSS growth:             {} MiB", rss_growth / (1024 * 1024));
    eprintln!("audit_outbound push/fail/pop: {}/{}/{}",
        diag::DIAG_AUDIT_OUTBOUND_PUSHES.load(Ordering::Relaxed),
        diag::DIAG_AUDIT_OUTBOUND_PUSH_FAILURES.load(Ordering::Relaxed),
        diag::DIAG_AUDIT_OUTBOUND_POPS.load(Ordering::Relaxed));
    eprintln!("audit_inbound push/fail/pop: {}/{}/{}",
        diag::DIAG_AUDIT_INBOUND_PUSHES.load(Ordering::Relaxed),
        diag::DIAG_AUDIT_INBOUND_PUSH_FAILURES.load(Ordering::Relaxed),
        diag::DIAG_AUDIT_INBOUND_POPS.load(Ordering::Relaxed));
    eprintln!("====================================================");

    assert!(elapsed < Duration::from_secs(30), "50 × 1 MiB transfers took {elapsed:?}");

    {
        use std::sync::atomic::Ordering::Relaxed;
        let acquires = diag::DIAG_RECV_POOL_ACQUIRES.load(Relaxed);
        let reused = diag::DIAG_RECV_POOL_REUSED.load(Relaxed);
        let fresh = diag::DIAG_RECV_POOL_FRESH_ALLOC.load(Relaxed);
        let released = diag::DIAG_RECV_POOL_RELEASED.load(Relaxed);
        let overflow = diag::DIAG_RECV_POOL_OVERFLOW_DROPPED.load(Relaxed);
        eprintln!(
            "RecvBufPool: acquires={acquires} reused={reused} fresh_alloc={fresh} \
             released={released} overflow_dropped={overflow} \
             in_flight={}",
            acquires.saturating_sub(released).saturating_sub(overflow),
        );
    }

    let total_payload = transfers * data.len();
    let threshold = total_payload + 80 * 1024 * 1024;
    assert!(
        rss_growth < threshold,
        "RSS grew by {} MiB over {transfers} transfers ({} MiB payload)",
        rss_growth / (1024 * 1024),
        total_payload / (1024 * 1024),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_streams_no_deadlock() {
    let f = connected_pair().await;
    let data = Arc::new(payload(1024 * 1024));

    let mut handles = Vec::new();
    for sid in 0..4u8 {
        let c = &f;
        let d = Arc::clone(&data);
        handles.push(async move {
            c.send_bulk(sid, &d, Duration::from_secs(30)).await
                .unwrap_or_else(|e| panic!("stream {sid} failed: {e}"));
        });
    }

    let start = Instant::now();
    futures::future::join_all(handles).await;
    let elapsed = start.elapsed();

    assert!(elapsed < Duration::from_secs(30), "4 concurrent 1 MiB streams took {elapsed:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn host_responsive_under_bulk_saturation() {
    let f = connected_pair().await;
    let data = payload(4 * 1024 * 1024);

    let stalled = Arc::new(AtomicBool::new(false));
    let max_gap_ns = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));

    let stalled_ref = Arc::clone(&stalled);
    let max_gap_ref = Arc::clone(&max_gap_ns);
    let done_ref = Arc::clone(&done);
    let watchdog = std::thread::spawn(move || {
        let mut last = Instant::now();
        while !done_ref.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            let now = Instant::now();
            let gap = now.duration_since(last);
            let gap_ns = gap.as_nanos() as u64;
            max_gap_ref.fetch_max(gap_ns, Ordering::Relaxed);
            if gap > Duration::from_millis(500) {
                stalled_ref.store(true, Ordering::Relaxed);
            }
            last = now;
        }
    });

    for i in 0..10 {
        f.send_bulk(0, &data, Duration::from_secs(30)).await
            .unwrap_or_else(|e| panic!("transfer {i} failed: {e}"));
    }

    done.store(true, Ordering::Relaxed);
    watchdog.join().unwrap();

    let max_gap = Duration::from_nanos(max_gap_ns.load(Ordering::Relaxed));
    assert!(
        !stalled.load(Ordering::Relaxed),
        "Host thread was starved for {max_gap:?} during bulk saturation"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn heartbeat_survives_bulk_saturation() {
    let config = IpcFixtureConfig {
        heartbeat_interval_ms: 200,
        heartbeat_miss_limit: 3,
        heartbeat_response_timeout_ms: 200,
        ..IpcFixtureConfig::for_test()
    };
    let f = connected_pair_with_config(config).await;

    let data = payload(4 * 1024 * 1024);

    for i in 0..10 {
        f.send_bulk(0, &data, Duration::from_secs(30)).await
            .unwrap_or_else(|e| panic!("transfer {i} failed under heartbeat pressure: {e}"));
    }

    f.send_request(b"alive", TEST_TIMEOUT).await
        .expect("session must still be alive after bulk saturation — heartbeat survived");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backpressure_prevents_oom() {
    diag::reset_all_diagnostics();
    let f = connected_pair().await;
    let data = payload(256 * 1024);

    for _ in 0..5 {
        f.send_bulk(0, &data, Duration::from_secs(30)).await
            .expect("warmup transfer failed");
    }
    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    unsafe { libc::malloc_trim(0); }
    let rss_before = get_rss_bytes();

    for i in 0..200 {
        f.send_bulk(0, &data, Duration::from_secs(30)).await
            .unwrap_or_else(|e| panic!("transfer {i} failed: {e}"));
    }

    let rss_after = get_rss_bytes();
    let rss_growth = rss_after.saturating_sub(rss_before);

    let total_payload = 200 * data.len();
    let threshold = total_payload + 80 * 1024 * 1024;
    assert!(
        rss_growth < threshold,
        "RSS grew by {} MiB over 200 transfers ({} MiB payload)",
        rss_growth / (1024 * 1024),
        total_payload / (1024 * 1024),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn datagrams_flow_during_bulk_transfer() {
    let f = connected_pair().await;
    let bulk_data = payload(8 * 1024 * 1024);
    let msg = b"ping during bulk";

    let bulk_fut = f.send_bulk(0, &bulk_data, Duration::from_secs(30));
    let datagram_fut = async {
        for i in 0..20 {
            f.send_request(msg, TEST_TIMEOUT).await
                .unwrap_or_else(|e| panic!("datagram {i} failed during bulk: {e}"));
        }
    };

    let (bulk_result, _) = tokio::join!(bulk_fut, datagram_fut);
    bulk_result.unwrap();
}
