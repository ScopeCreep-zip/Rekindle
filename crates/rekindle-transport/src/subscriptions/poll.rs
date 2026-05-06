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

                let mut records_with_changes = 0u32;
                let mut total_inspected = 0u32;
                let mut total_populated = 0u32;
                let mut total_read = 0u32;
                let mut inspect_failures = 0u32;

                for (record_key, subkeys) in &entries {
                    total_inspected += 1;
                    let record_start = Instant::now();

                    // NOTE: inspect() + get() is not atomic (benign TOCTOU). Between
                    // inspect and get, a subkey could be written (false negative — caught
                    // by next sweep) or cleared (stale read — harmless). Tier 1 (watch)
                    // and tier 2 (direct notification) provide sub-second delivery; this
                    // poll loop is the guaranteed fallback, not the primary path.

                    // Step 1: Inspect — one network call for all subkeys
                    let populated = match crate::broadcast::dht_writes::inspect(
                        &node, record_key, Some(subkeys),
                    ).await {
                        Ok(report) => {
                            let pop: Vec<u32> = report.subkeys().iter()
                                .zip(report.local_seqs().iter())
                                .filter(|(_, seq)| seq.is_some())
                                .map(|(sk, _)| sk)
                                .collect();
                            debug!(
                                record_key = &record_key[..20.min(record_key.len())],
                                populated = pop.len(),
                                total_subkeys = subkeys.len(),
                                inspect_ms = record_start.elapsed().as_millis(),
                                "poll: inspect complete"
                            );
                            pop
                        }
                        Err(e) => {
                            warn!(
                                record_key = &record_key[..20.min(record_key.len())],
                                error = %e,
                                "poll: inspect failed, falling back to full read (SLOW)"
                            );
                            inspect_failures += 1;
                            subkeys.clone()
                        }
                    };

                    if populated.is_empty() {
                        continue;
                    }

                    #[allow(clippy::cast_possible_truncation)]
                    { total_populated += populated.len() as u32; }

                    // Step 2: Read only populated subkeys with force_refresh
                    let mut changed_subkeys = Vec::new();
                    for &subkey in &populated {
                        total_read += 1;
                        match crate::broadcast::dht_writes::get(&node, record_key, subkey, true).await {
                            Ok(Some(data)) if !data.is_empty() => {
                                changed_subkeys.push(subkey);
                            }
                            Ok(_) => {}
                            Err(e) => {
                                debug!(
                                    record_key = &record_key[..20.min(record_key.len())],
                                    subkey, error = %e,
                                    "poll: get failed"
                                );
                            }
                        }
                    }

                    // Step 3: Signal changes
                    if !changed_subkeys.is_empty() {
                        records_with_changes += 1;
                        debug!(
                            record_key = &record_key[..20.min(record_key.len())],
                            changed = changed_subkeys.len(),
                            record_ms = record_start.elapsed().as_millis(),
                            "poll: record has changes, signaling"
                        );
                        if change_tx.send((record_key.clone(), changed_subkeys)).await.is_err() {
                            info!("poll: change channel closed, exiting");
                            return;
                        }
                    }
                }

                info!(
                    records_inspected = total_inspected,
                    records_with_changes,
                    total_populated,
                    total_read,
                    inspect_failures,
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
