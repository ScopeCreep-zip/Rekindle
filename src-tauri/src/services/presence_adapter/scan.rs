//! Per-segment raw-bytes fetch for the registry scan.
//!
//! Thin Veilid-IO helper over `RecordPool::read_changed` (plan C7.12): each
//! tick runs ONE authoritative `inspect_dht_record(UpdateGet)` — a
//! network-fanout view of every subkey's local vs. network sequence number —
//! then re-reads from the network only the subkeys whose network seq is
//! newer than our local copy. Returns `(subkey, raw_bytes)` for every
//! populated entry. The pure
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

use crate::state::AppState;
use crate::state_helpers;

pub(super) async fn scan_segment_raw(
    state: &Arc<AppState>,
    registry_key: &str,
    max_subkey: u32,
    skip_subkey: Option<u32>,
) -> Vec<(u32, Vec<u8>)> {
    let Ok(pool) = state_helpers::record_pool(state) else {
        tracing::trace!(registry_key, "scan_segment_raw: logged out — skipping");
        return Vec::new();
    };
    let Ok(reg_key) = registry_key.parse::<veilid_core::RecordKey>() else {
        tracing::warn!(registry_key, "scan_segment_raw: invalid registry key");
        return Vec::new();
    };

    // One borrow for the whole scan (a table hit: the community holds its
    // registry), then one authoritative network-fanout inspect over the
    // segment range, which yields the local-vs-network seq vectors we diff
    // below.
    let lease = match pool.acquire(&reg_key, None).await {
        Ok(lease) => lease,
        Err(error) => {
            tracing::debug!(registry_key, %error, "scan_segment_raw: registry not open — skipping");
            return Vec::new();
        }
    };
    let out = scan_leased(&pool, lease, registry_key, max_subkey, skip_subkey).await;
    pool.release(lease).await;
    out
}

/// [`scan_segment_raw`] on the registry's lease: the registry is the
/// membership index itself, so its whole range is the read
/// (`RecordPool::read_changed`, plan C7.12).
async fn scan_leased(
    pool: &rekindle_protocol::dht::pool::RecordPool,
    lease: rekindle_records::lease::LeaseId,
    registry_key: &str,
    max_subkey: u32,
    skip_subkey: Option<u32>,
) -> Vec<(u32, Vec<u8>)> {
    let subkeys: Vec<u32> = (0..max_subkey)
        .filter(|subkey| Some(*subkey) != skip_subkey)
        .collect();
    match pool.read_changed(lease, &subkeys).await {
        Ok(values) => values
            .into_iter()
            .filter(|(_, value)| !value.data().is_empty())
            .map(|(subkey, value)| (subkey, value.data().to_vec()))
            .collect(),
        Err(error) => {
            tracing::debug!(
                registry_key,
                %error,
                "scan_segment_raw: registry not inspectable — skipping scan this tick",
            );
            Vec::new()
        }
    }
}
