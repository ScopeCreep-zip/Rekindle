//! Phase 20 REDO — pure reliability-weighted peer selection.
//!
//! Architecture §14.5 (Mutual Aid): rank gossip candidates by
//! `success / (success + failure)` so high-reliability peers
//! ("ziplines") are preferred. Peers with no metrics get a neutral
//! score (0.5) so new joiners aren't permanently shut out.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::BuildHasher;

use crate::deps::PeerInfo;

/// Sort peers by reliability score (descending). Ties broken by
/// pseudonym key (ascending) for determinism across nodes.
#[must_use]
pub fn sort_peers_by_reliability<S: BuildHasher>(
    peers: Vec<PeerInfo>,
    scores: &HashMap<String, f64, S>,
) -> Vec<PeerInfo> {
    let mut scored: Vec<(f64, PeerInfo)> = peers
        .into_iter()
        .map(|peer| {
            let score = scores.get(&peer.pseudonym_key).copied().unwrap_or(0.5);
            (score, peer)
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.1.pseudonym_key.cmp(&b.1.pseudonym_key))
    });
    scored.into_iter().map(|(_, peer)| peer).collect()
}

/// Convert raw (success, failure) counter tuples into reliability
/// scores in `[0.0, 1.0]`. Mirrors the in-memory map src-tauri
/// keeps on `CommunityState.peer_reliability`.
#[must_use]
pub fn scores_from_counters<S: BuildHasher>(
    counters: &HashMap<String, (u32, u32), S>,
) -> HashMap<String, f64> {
    counters
        .iter()
        .map(|(peer, &(succ, fail))| {
            let total = f64::from(succ) + f64::from(fail);
            let score = if total <= 0.0 {
                0.5
            } else {
                f64::from(succ) / total
            };
            (peer.clone(), score)
        })
        .collect()
}

#[cfg(test)]
#[path = "peer_select_tests.rs"]
mod tests;
