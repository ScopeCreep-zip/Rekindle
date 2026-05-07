//! Join inbox reading — inspect-optimized scan of pending join requests.

use tracing::info;

use crate::error::Result;
use crate::payload::dht_types::{PendingJoinEntry, PendingJoinStatus};

/// Read all actionable join requests from a community inbox.
///
/// Uses inspect() to identify populated subkeys in one network call,
/// then reads only those subkeys. Handles both array and single-entry
/// formats for backward compatibility.
pub async fn read_inbox_requests(dht: &crate::broadcast::dht::DhtStore, inbox_key: &str) -> Result<Vec<PendingJoinEntry>> {
    let start = std::time::Instant::now();

    let all_subkeys: Vec<u32> = (0..crate::payload::dht_types::JOIN_INBOX_SUBKEY_COUNT).collect();
    let populated = match crate::broadcast::dht::record::inspect(dht.routing_context(), inbox_key, Some(&all_subkeys)).await {
        Ok(report) => {
            let pop: Vec<u32> = report.subkeys().iter()
                .zip(report.local_seqs().iter())
                .filter(|(_, seq)| seq.is_some())
                .map(|(sk, _)| sk)
                .collect();
            info!(
                populated = pop.len(),
                total = crate::payload::dht_types::JOIN_INBOX_SUBKEY_COUNT,
                inspect_ms = start.elapsed().as_millis(),
                "read_inbox_requests: inspect found {} populated subkeys", pop.len()
            );
            pop
        }
        Err(e) => {
            tracing::warn!(error = %e, "read_inbox_requests: inspect failed, falling back to full scan (SLOW)");
            all_subkeys
        }
    };

    if populated.is_empty() {
        info!(elapsed_ms = start.elapsed().as_millis(), "read_inbox_requests: inbox empty");
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let mut read_errors = 0u32;
    for subkey in &populated {
        let data = match tokio::time::timeout(
            std::time::Duration::from_millis(5000),
            crate::broadcast::dht::record::get(dht.routing_context(), inbox_key, *subkey, false),
        ).await {
            Ok(Ok(Some(d))) if !d.is_empty() && d != b"[]" => d,
            Ok(Err(e)) => {
                tracing::debug!(subkey, error = %e, "read_inbox_requests: get failed");
                read_errors += 1;
                continue;
            }
            _ => continue,
        };
        let parsed: Vec<PendingJoinEntry> = match serde_json::from_slice::<Vec<PendingJoinEntry>>(&data) {
            Ok(arr) => arr,
            Err(_) => match serde_json::from_slice::<PendingJoinEntry>(&data) {
                Ok(single) => vec![single],
                Err(e) => {
                    tracing::debug!(subkey, error = %e, "read_inbox_requests: parse failed");
                    continue;
                }
            },
        };
        for entry in parsed {
            if matches!(entry.status, PendingJoinStatus::Pending | PendingJoinStatus::Left { .. }) {
                info!(requester = %entry.display_name, subkey, status = ?entry.status, "read_inbox_requests: found entry");
                entries.push(entry);
            }
        }
    }

    info!(entries = entries.len(), subkeys_read = populated.len(), read_errors, elapsed_ms = start.elapsed().as_millis(), "read_inbox_requests: complete");
    Ok(entries)
}
