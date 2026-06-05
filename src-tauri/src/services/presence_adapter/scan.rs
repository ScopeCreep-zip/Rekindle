//! Per-segment raw-bytes fetch for the registry scan.
//!
//! Thin Veilid-IO helper. Each tick runs ONE authoritative
//! `inspect_dht_record(UpdateGet)` — a network-fanout view of every
//! subkey's local vs. network sequence number — then forces a network
//! re-read (`get_dht_value(force_refresh = true)`) only for the subkeys
//! whose network seq is newer than (or absent from) our local cache.
//! Returns `(subkey, raw_bytes)` for every populated entry. The pure
//! business logic — W26 signature verify, ban filter, heartbeat
//! classification — lives in
//! `crates/rekindle-presence/src/community/scan_row.rs` per Invariant 7
//! (src-tauri carries no protocol logic).
//!
//! Why inspect-then-force-refresh and not a plain `get_dht_value(.., false)`:
//! Veilid's `get_value` short-circuits to the LOCAL CACHE the instant any
//! value exists for a subkey and never re-reads the network (veilid-core
//! 0.5.2 `storage_manager/get_value.rs`). So a peer whose first heartbeat we
//! cached once would appear frozen — every later heartbeat invisible — until
//! the classifier ages them out to "offline", even though they are live and
//! re-publishing every tick. Relying on the registry watch to refresh that
//! cache is not enough: watches are best-effort, fail at startup, and do not
//! backfill pre-existing values. This is the classic anti-entropy rule — a
//! live subscription (watch) MUST be paired with periodic full-state
//! reconciliation (Dynamo Merkle anti-entropy, SSB seq-vector backfill,
//! Matrix full-state /sync). `inspect(UpdateGet)` is Veilid's native seq-diff
//! primitive for exactly this; we pull only the subkeys that actually moved.

use std::sync::Arc;

use futures::stream::{FuturesUnordered, StreamExt};

use crate::state::AppState;
use crate::state_helpers;

/// Concurrent get_dht_value calls per scan. Mirrors pre-port poll.rs.
const SCAN_PARALLELISM: usize = 10;

pub(super) async fn scan_segment_raw(
    state: &Arc<AppState>,
    registry_key: &str,
    max_subkey: u32,
    skip_subkey: Option<u32>,
) -> Vec<(u32, Vec<u8>)> {
    let Some(rc) = state_helpers::safe_routing_context(state) else {
        tracing::trace!(registry_key, "scan_segment_raw: not attached — skipping");
        return Vec::new();
    };
    let Ok(reg_key) = registry_key.parse::<veilid_core::RecordKey>() else {
        tracing::warn!(registry_key, "scan_segment_raw: invalid registry key");
        return Vec::new();
    };

    // One authoritative network-fanout inspect over the whole segment range.
    // This both probes that the record is open (a single `Generic: record not
    // open` instead of 255 of them — self-healing, the next tick re-opens) and
    // yields the local-vs-network seq vectors we diff below. `inspect` requires
    // an opened record, which the caller's `ensure_registry_open` guarantees.
    let inspect_range =
        veilid_core::ValueSubkeyRangeSet::single_range(0, max_subkey.saturating_sub(1));
    let report = match rc
        .inspect_dht_record(
            reg_key.clone(),
            Some(inspect_range),
            veilid_core::DHTReportScope::UpdateGet,
        )
        .await
    {
        Ok(report) => report,
        Err(error) => {
            tracing::debug!(
                registry_key,
                %error,
                "scan_segment_raw: registry not inspectable — skipping scan this tick",
            );
            return Vec::new();
        }
    };

    // Diff network vs local seq per subkey. `ValueSeqNum::to_option()` is
    // `None` when the subkey is empty; ordering on `Option<u32>` treats
    // `None` < `Some(_)`, so `network > local` is true both when a peer is
    // newly present (local `None`) and when their heartbeat advanced the seq.
    // Only those subkeys need a forced network re-read; everything else is
    // already current in cache and a cheap local read suffices.
    let local_seqs = report.local_seqs();
    let network_seqs = report.network_seqs();
    let subkeys = report.subkeys();

    let sem = Arc::new(tokio::sync::Semaphore::new(SCAN_PARALLELISM));
    let mut futs = FuturesUnordered::new();
    for (i, network_seq) in network_seqs.iter().enumerate() {
        let Some(subkey) = subkeys.nth_subkey(i) else {
            continue;
        };
        if Some(subkey) == skip_subkey {
            continue;
        }
        let network = network_seq.to_option();
        if network.is_none() {
            // No value on the network for this subkey — nothing to fetch.
            continue;
        }
        let local = local_seqs
            .get(i)
            .and_then(veilid_core::ValueSeqNum::to_option);
        let force_refresh = network > local;

        let sem = Arc::clone(&sem);
        let rc = rc.clone();
        let rk = reg_key.clone();
        futs.push(async move {
            let permit = sem.acquire().await.expect("semaphore closed");
            let result = rc.get_dht_value(rk, subkey, force_refresh).await;
            drop(permit);
            (subkey, result)
        });
    }

    let mut out = Vec::new();
    while let Some((subkey, result)) = futs.next().await {
        let Ok(Some(val)) = result else { continue };
        let bytes = val.data().to_vec();
        if bytes.is_empty() {
            continue;
        }
        out.push((subkey, bytes));
    }
    out
}
