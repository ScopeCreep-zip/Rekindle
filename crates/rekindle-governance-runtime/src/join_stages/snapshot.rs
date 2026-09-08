//! Loading the multi-segment governance snapshot.
//!
//! Split out of `join_stages/mod.rs` at the natural seam: this half
//! answers *what does governance currently say*, while the rest of the
//! module answers *which slot do I take*. They share only the snapshot
//! type, and the claim state machine is the part that grows.

use rekindle_governance::merge;
use rekindle_governance::state::GovernanceState;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::join::default_community_name;

/// Snapshot of the multi-segment governance record set, ready for the
/// adapter to build a fresh `CommunityState`.
pub struct GovernanceSnapshot {
    pub all_entries: Vec<(PseudonymKey, Vec<GovernanceEntry>)>,
    pub gov_state: GovernanceState,
    pub name: String,
    pub description: Option<String>,
    /// Every GovernanceOverflow record key followed while loading this
    /// snapshot (primary + all segments). The src-tauri join flow seeds these
    /// into the new `CommunityState.open_community_records.governance_overflow_keys`
    /// so they are opened+tracked (§10) and warmed/rehydrated (§14.1) like every
    /// other community record — the joiner can't register via `community_id`
    /// here because the community isn't in `AppState` yet.
    pub overflow_keys: Vec<String>,
}

/// First-pass + multi-segment governance snapshot: fetch + W26-verify
/// every signed `GovernanceSubkeyPayload` from the primary record, merge
/// to find announced segments, fetch + verify each segment, re-merge.
pub async fn load_governance_snapshot<D: GovernanceRuntimeDeps>(
    deps: &D,
    governance_key_str: &str,
) -> Result<GovernanceSnapshot, GovernanceRuntimeError> {
    let primary = fetch_governance_record_entries(deps, governance_key_str).await?;
    let mut all_entries = primary.authored;
    let mut overflow_keys = primary.overflow_keys;
    let gov_state_v1 = merge::merge(&all_entries);

    // Pass 2: fetch every segment's governance record. CRDT idempotence +
    // commutativity (Almeida 2016 §3) means re-merge is canonical
    // regardless of fetch order.
    for segment in &gov_state_v1.segments {
        if segment.segment_index == 0 {
            continue;
        }
        match fetch_governance_record_entries(deps, &segment.governance_key).await {
            Ok(mut extra) => {
                all_entries.append(&mut extra.authored);
                overflow_keys.append(&mut extra.overflow_keys);
            }
            Err(e) => tracing::warn!(
                segment = segment.segment_index,
                governance_key = %segment.governance_key,
                error = %e,
                "load_governance_snapshot: skipping unreachable segment governance record"
            ),
        }
    }
    let gov_state = if gov_state_v1.segments.iter().any(|s| s.segment_index > 0) {
        merge::merge(&all_entries)
    } else {
        gov_state_v1
    };

    let name = gov_state.metadata.as_ref().map_or_else(
        || default_community_name(governance_key_str),
        |metadata| metadata.name.clone(),
    );
    let description = gov_state
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.description.clone());

    overflow_keys.sort();
    overflow_keys.dedup();

    Ok(GovernanceSnapshot {
        all_entries,
        gov_state,
        name,
        description,
        overflow_keys,
    })
}

/// Subkeys an `UpdateGet` inspect reports as holding a value, or the full
/// `0..255` range when the inspect call fails.
///
/// A blind `0..255` `get_dht_value` sweep costs 255 serial network
/// round-trips on a cold join — the local store is empty until each subkey
/// is fetched, and ~250 of those land on empty slots, enough to exceed the
/// UI's join timeout. One inspect collapses that to a single network
/// round-trip plus a fetch per populated subkey. On inspect error we fall
/// back to the full sweep so a join never silently drops entries.
async fn populated_subkeys_or_full_scan<D: GovernanceRuntimeDeps>(
    deps: &D,
    record_key: &str,
) -> Vec<u32> {
    match deps.inspect_dht_record_present_subkeys(record_key).await {
        Ok(subkeys) => subkeys,
        Err(e) => {
            tracing::warn!(
                record = %record_key,
                error = %e,
                "inspect failed; falling back to full 0..255 subkey scan"
            );
            (0..255u32).collect()
        }
    }
}

/// Read the populated subkeys of a single governance record, W26-verify each
/// payload's pseudonym signature (any member could write to any slot, so the
/// signature is the only authorship proof), and follow every author's
/// `overflow_next` chain so a spilled author's log is reassembled in full
/// before merge (architecture §"Follow GovernanceOverflow pointers", line 1609).
async fn fetch_governance_record_entries<D: GovernanceRuntimeDeps>(
    deps: &D,
    governance_key_str: &str,
) -> Result<crate::overflow::GovernanceReadout, GovernanceRuntimeError> {
    deps.open_dht_record(governance_key_str, None).await?;
    let occupied = populated_subkeys_or_full_scan(deps, governance_key_str).await;
    Ok(crate::overflow::read_governance_with_overflow(deps, governance_key_str, &occupied).await)
}
