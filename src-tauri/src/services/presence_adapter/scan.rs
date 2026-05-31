//! Per-segment raw-bytes fetch for the registry scan.
//!
//! Thin Veilid-IO helper: pumps `0..max_subkey` through a
//! `SCAN_PARALLELISM`-permit semaphore + per-subkey
//! `get_dht_value`, returning `(subkey, raw_bytes)` for every
//! populated entry. The pure business logic — W26 signature verify,
//! ban filter, heartbeat classification — lives in
//! `crates/rekindle-presence/src/community/scan_row.rs` per
//! Invariant 7 (src-tauri carries no protocol logic).

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

    let sem = Arc::new(tokio::sync::Semaphore::new(SCAN_PARALLELISM));
    let mut futs = FuturesUnordered::new();
    for subkey in 0..max_subkey {
        if Some(subkey) == skip_subkey {
            continue;
        }
        let sem = Arc::clone(&sem);
        let rc = rc.clone();
        let rk = reg_key.clone();
        futs.push(async move {
            let permit = sem.acquire().await.expect("semaphore closed");
            let result = rc.get_dht_value(rk, subkey, false).await;
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
