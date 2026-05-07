//! Tier 3 periodic DHT poll — guaranteed fallback for missed watches and gossip.
//!
//! Sweeps all watched DHT records at a configurable interval (default 60s).
//! Uses `inspect()` to identify populated subkeys in one network call per record,
//! then reads only those subkeys with `force_refresh=true`. This avoids the
//! 32 × 10s sequential read penalty for empty subkeys.
//!
//! When changes are found, signals the SubscriptionManager via a channel so
//! events flow through the broadcast pipeline to daemon-internal consumers
//! (process_inbox, friend inbox scan) and IPC clients (TUI rendering).

use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::broadcast::node::TransportNode;

use super::watches::WatchRegistry;

/// Run the tier 3 poll loop. Sweeps all watched records periodically.
///
/// For each record:
/// 1. `inspect()` — one network call returns subkey sequence numbers
/// 2. Filter to subkeys with data (seq.is_some())
/// 3. `get()` only populated subkeys with `force_refresh=true`
/// 4. Signal changed subkeys on `change_tx`
///
/// This reduces a 32-subkey inbox poll from ~320s (32 × 10s timeouts)
/// to ~1-5s (1 inspect + N populated reads).
pub async fn run_poll_loop(
    node: Arc<TransportNode>,
    watches: Arc<RwLock<WatchRegistry>>,
    interval_secs: u64,
    change_tx: mpsc::Sender<(String, Vec<u32>)>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    interval.tick().await; // skip immediate first tick

    info!(interval_secs, "poll loop started (tier 3 fallback)");

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if !node.is_ready() {
                    debug!("poll: suppressed (node not ready)");
                    continue;
                }

                let sweep_start = Instant::now();

                let entries: Vec<(String, Vec<u32>)> = {
                    let reg = watches.read();
                    reg.entries.iter().map(|(k, e)| {
                        (k.clone(), e.subkeys.clone())
                    }).collect()
                };

                if entries.is_empty() {
                    debug!("poll: no watched records");
                    continue;
                }

                const MAX_CONCURRENT_POLLS: usize = 8;

                let mut records_with_changes = 0u32;
                #[allow(clippy::cast_possible_truncation)]
                let total_inspected = entries.len() as u32;
                let mut join_set = tokio::task::JoinSet::new();
                let mut entries_iter = entries.into_iter();
                let mut pending = 0usize;

                loop {
                    while pending < MAX_CONCURRENT_POLLS {
                        let Some((record_key, subkeys)) = entries_iter.next() else { break };
                        let n = Arc::clone(&node);
                        join_set.spawn(async move {
                            poll_single_record(&n, &record_key, &subkeys).await
                        });
                        pending += 1;
                    }

                    if pending == 0 { break; }

                    match join_set.join_next().await {
                        Some(Ok(Some((record_key, changed)))) => {
                            records_with_changes += 1;
                            if change_tx.send((record_key, changed)).await.is_err() {
                                info!("poll: change channel closed, exiting");
                                return;
                            }
                        }
                        Some(Ok(None)) => {} // no changes
                        Some(Err(e)) => { warn!(error = %e, "poll: task panic"); }
                        None => break,
                    }
                    pending -= 1;
                }

                info!(
                    records_inspected = total_inspected,
                    records_with_changes,
                    concurrent_max = MAX_CONCURRENT_POLLS,
                    sweep_ms = sweep_start.elapsed().as_millis(),
                    "poll: sweep complete"
                );
            }
            _ = shutdown_rx.recv() => {
                info!("poll loop shutting down");
                break;
            }
        }
    }
}

/// Poll a single DHT record: inspect → filter populated → get(force_refresh).
///
/// Returns `None` if no changes, `Some((record_key, changed_subkeys))` if changes found.
async fn poll_single_record(
    node: &TransportNode,
    record_key: &str,
    subkeys: &[u32],
) -> Option<(String, Vec<u32>)> {
    let populated = match crate::broadcast::dht_writes::inspect(node, record_key, Some(subkeys)).await {
        Ok(report) => {
            report.subkeys().iter()
                .zip(report.local_seqs().iter())
                .filter(|(_, seq)| seq.is_some())
                .map(|(sk, _)| sk)
                .collect::<Vec<u32>>()
        }
        Err(e) => {
            warn!(record_key = &record_key[..20.min(record_key.len())], error = %e, "poll: inspect failed, fallback to full read");
            subkeys.to_vec()
        }
    };

    if populated.is_empty() { return None; }

    let mut changed = Vec::new();
    for &subkey in &populated {
        match crate::broadcast::dht_writes::get(node, record_key, subkey, true).await {
            Ok(Some(data)) if !data.is_empty() => changed.push(subkey),
            _ => {}
        }
    }

    if changed.is_empty() { None } else { Some((record_key.to_string(), changed)) }
}
