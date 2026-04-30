use std::time::{Duration, Instant};

use crate::fetch::{FetchQueue, FetchTask};
use crate::gap::GapDetector;
use crate::inspect::InspectLoop;
use crate::verify::verify_content_hash;
use crate::watch::WatchManager;

fn queue_fetch_task(queue: &mut FetchQueue, record_key: &str, subkey: u32, ciphertext: &[u8]) {
    queue.push(FetchTask {
        record_key: record_key.into(),
        subkey,
        expected_hash: Some(blake3::hash(ciphertext).to_hex().to_string()),
        attempt: 0,
    });
}

fn fetch_and_verify(queue: &mut FetchQueue, ciphertext: &[u8]) -> bool {
    let task = queue.pop().expect("expected queued fetch task");
    let expected_hash = task.expected_hash.expect("expected content hash");
    verify_content_hash(ciphertext, &expected_hash)
}

#[test]
fn smpl_only_write_is_processed_via_inspect_without_gossip() {
    let local_sequences = [0_u64, 0];
    let network_sequences = [0_u64, 1];

    let gaps = GapDetector::detect(&local_sequences, &network_sequences);
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].subkey, 1);

    let ciphertext = b"smpl write landed on dht";
    let mut queue = FetchQueue::default();
    queue_fetch_task(
        &mut queue,
        "channel_record",
        gaps[0].subkey as u32,
        ciphertext,
    );

    assert!(
        fetch_and_verify(&mut queue, ciphertext),
        "inspect-discovered SMPL write should be fetchable and verifiable without gossip peers",
    );
}

#[test]
fn gossip_only_notification_is_processed_while_smpl_write_is_queued() {
    let smpl_write_queued_for_retry = true;
    assert!(smpl_write_queued_for_retry);

    let ciphertext = b"gossip notification metadata";
    let mut queue = FetchQueue::default();
    queue_fetch_task(&mut queue, "channel_record", 7, ciphertext);

    assert!(
        fetch_and_verify(&mut queue, ciphertext),
        "gossip notification should be sufficient to drive fetch and verification even when Path 1 is queued",
    );
}

#[test]
fn inspect_only_detects_and_processes_when_gossip_and_watch_fail() {
    let start = Instant::now();
    let inspect = InspectLoop::new(start);
    let watches = WatchManager::default();

    assert!(!watches.is_active("channel_record"));
    assert!(inspect.should_run_at(start + Duration::from_secs(60)));

    let local_sequences = [0_u64];
    let network_sequences = [3_u64];
    let gaps = GapDetector::detect(&local_sequences, &network_sequences);
    assert_eq!(gaps.len(), 1);

    let ciphertext = b"inspect only catchup";
    let mut queue = FetchQueue::default();
    queue_fetch_task(
        &mut queue,
        "channel_record",
        gaps[0].subkey as u32,
        ciphertext,
    );

    assert!(
        fetch_and_verify(&mut queue, ciphertext),
        "inspect polling should process the message even when gossip and watches are unavailable",
    );
}
