//! Pure helpers used by the community-presence orchestrators.
//!
//! Decomposed out of `presence_poll_tick` so the orchestrator stays
//! under the per-file LoC cap (Invariant 1).

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use rand::seq::SliceRandom;
use rekindle_types::id::RoleId;

/// Derive a 16-byte presence-event identifier from a UTF-8 event-id
/// string via BLAKE3 (truncate). Used both when publishing our own
/// RSVPs and when matching incoming RSVPs back to events.
#[must_use]
pub fn presence_event_id_bytes(event_id: &str) -> [u8; 16] {
    let hash = blake3::hash(event_id.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash.as_bytes()[..16]);
    bytes
}

/// Convert a governance-state set of `RoleId`s into the sorted
/// `Vec<u32>` shape the in-memory member roles map uses. Stable
/// ordering keeps SQLite role_ids_json deterministic.
#[must_use]
pub fn role_ids_from_governance<S: BuildHasher>(assigned: &HashSet<RoleId, S>) -> Vec<u32> {
    let mut ids: Vec<u32> = assigned
        .iter()
        .map(|role_id| u32::from_le_bytes([role_id.0[0], role_id.0[1], role_id.0[2], role_id.0[3]]))
        .collect();
    ids.sort_unstable();
    ids
}

/// Architecture §3 — pick a uniformly-random `d`-sized subset of
/// the online members for the gossip fan-out. Returns the full set
/// when `d >= online.len()` (no point in random selection).
#[must_use]
pub fn random_peer_sample<V, S>(online: &HashMap<String, V, S>, d: usize) -> HashMap<String, V>
where
    V: Clone,
    S: BuildHasher,
{
    if d == 0 || online.is_empty() {
        return HashMap::new();
    }
    if d >= online.len() {
        return online.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    }
    let keys: Vec<&String> = online.keys().collect();
    let mut rng = rand::rngs::OsRng;
    let selected: Vec<&String> = keys.choose_multiple(&mut rng, d).copied().collect();
    selected
        .into_iter()
        .filter_map(|k| online.get(k).map(|v| (k.clone(), v.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_event_id_bytes_is_deterministic() {
        let a = presence_event_id_bytes("event-42");
        let b = presence_event_id_bytes("event-42");
        assert_eq!(a, b);
        let c = presence_event_id_bytes("event-43");
        assert_ne!(a, c);
    }

    #[test]
    fn role_ids_from_governance_sorts_ascending() {
        let mut set: HashSet<RoleId> = HashSet::new();
        let role = |first: u8| {
            let mut bytes = [0u8; 16];
            bytes[0] = first;
            RoleId(bytes)
        };
        set.insert(role(7));
        set.insert(role(2));
        set.insert(role(5));
        let ids = role_ids_from_governance(&set);
        assert_eq!(ids, vec![2, 5, 7]);
    }

    #[test]
    fn role_ids_from_governance_empty_returns_empty() {
        let set: HashSet<RoleId> = HashSet::new();
        assert!(role_ids_from_governance(&set).is_empty());
    }

    #[test]
    fn random_peer_sample_empty_set_returns_empty() {
        let online: HashMap<String, u32> = HashMap::new();
        assert!(random_peer_sample(&online, 5).is_empty());
    }

    #[test]
    fn random_peer_sample_zero_degree_returns_empty() {
        let mut online: HashMap<String, u32> = HashMap::new();
        online.insert("alice".into(), 1);
        online.insert("bob".into(), 2);
        assert!(random_peer_sample(&online, 0).is_empty());
    }

    #[test]
    fn random_peer_sample_full_when_degree_at_or_above_size() {
        let mut online: HashMap<String, u32> = HashMap::new();
        online.insert("alice".into(), 1);
        online.insert("bob".into(), 2);
        let s = random_peer_sample(&online, 2);
        assert_eq!(s.len(), 2);
        let s = random_peer_sample(&online, 99);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn random_peer_sample_subsets_to_exact_degree() {
        let mut online: HashMap<String, u32> = HashMap::new();
        for i in 0..10u32 {
            online.insert(format!("p{i}"), i);
        }
        let s = random_peer_sample(&online, 3);
        assert_eq!(s.len(), 3);
        // Every returned key must exist in the source.
        for key in s.keys() {
            assert!(online.contains_key(key));
        }
    }
}
