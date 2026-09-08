//! Reading a member registry's occupied slots.
//!
//! The expensive part of every registry question is the same: one
//! `get_dht_value` per occupied subkey. What differs is the predicate
//! applied afterwards — [`super::reclaim`] asks "is this slot free to
//! take", a roster asks "is this a member". Factoring the fetch keeps
//! one description of what a scan costs, and stops a second caller from
//! quietly reintroducing a second sweep.
//!
//! Classification is deliberately *not* done here. It needs the
//! community's ban set, and returning raw rows lets each caller decide
//! its own rule while sharing the I/O.

use crate::deps::GovernanceRuntimeDeps;

/// Fetch every occupied slot's raw bytes, lowest subkey first.
///
/// `occupied` comes from `inspect_dht_record_present_subkeys`, which
/// reports *ever written* rather than *currently occupied* — Veilid has
/// no per-subkey delete — so a returned row may well be a tombstone or
/// a banned member's. That is the caller's business, not this
/// function's.
///
/// A slot that fails to fetch is **omitted rather than defaulted**. A
/// transient DHT error is not evidence about the slot's contents, and
/// both callers would draw a wrong conclusion from an empty stand-in:
/// reclamation would hand a live member's slot away, a roster would
/// under-count.
pub(crate) async fn fetch_occupied<D: GovernanceRuntimeDeps>(
    deps: &D,
    registry_key: &str,
    occupied: &[u32],
) -> Vec<(u32, Vec<u8>)> {
    let mut rows = Vec::with_capacity(occupied.len());
    for &subkey in occupied {
        if let Ok(Some(raw)) = deps.get_dht_value(registry_key, subkey, false).await {
            rows.push((subkey, raw));
        }
    }
    rows.sort_unstable_by_key(|(subkey, _)| *subkey);
    rows
}
