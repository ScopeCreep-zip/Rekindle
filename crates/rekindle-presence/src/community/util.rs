//! Pure helpers used by the community-presence orchestrators.
//!
//! Decomposed out of `presence_poll_tick` so the orchestrator stays
//! under the per-file LoC cap (Invariant 1).

use std::collections::HashMap;
use std::hash::BuildHasher;

use rekindle_utils::RandomSource;

/// Architecture §3 — pick a uniformly-random `d`-sized subset of
/// the online members for the gossip fan-out, drawn from `rng` (the
/// host's RNG port, plan C9.2) over the members in key order, so a seed
/// fixes the subset. Returns the full set when `d >= online.len()` (no
/// point in random selection).
#[must_use]
pub fn random_peer_sample<V, S>(
    online: &HashMap<String, V, S>,
    d: usize,
    rng: &dyn RandomSource,
) -> HashMap<String, V>
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
    let mut keys: Vec<&String> = online.keys().collect();
    keys.sort_unstable();
    // A partial Fisher-Yates shuffle: the first `d` slots are the sample.
    for i in 0..d {
        let remaining = (keys.len() - i) as u64;
        let pick = i + usize::try_from(rng.below(remaining)).unwrap_or(0);
        keys.swap(i, pick);
    }
    keys[..d]
        .iter()
        .filter_map(|k| online.get(*k).map(|v| ((*k).clone(), v.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_utils::OsRandom;

    #[test]
    fn random_peer_sample_empty_set_returns_empty() {
        let online: HashMap<String, u32> = HashMap::new();
        assert!(random_peer_sample(&online, 5, &OsRandom).is_empty());
    }

    #[test]
    fn random_peer_sample_zero_degree_returns_empty() {
        let mut online: HashMap<String, u32> = HashMap::new();
        online.insert("alice".into(), 1);
        online.insert("bob".into(), 2);
        assert!(random_peer_sample(&online, 0, &OsRandom).is_empty());
    }

    #[test]
    fn random_peer_sample_full_when_degree_at_or_above_size() {
        let mut online: HashMap<String, u32> = HashMap::new();
        online.insert("alice".into(), 1);
        online.insert("bob".into(), 2);
        let s = random_peer_sample(&online, 2, &OsRandom);
        assert_eq!(s.len(), 2);
        let s = random_peer_sample(&online, 99, &OsRandom);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn random_peer_sample_subsets_to_exact_degree() {
        let mut online: HashMap<String, u32> = HashMap::new();
        for i in 0..10u32 {
            online.insert(format!("p{i}"), i);
        }
        let s = random_peer_sample(&online, 3, &OsRandom);
        assert_eq!(s.len(), 3);
        // Every returned key must exist in the source.
        for key in s.keys() {
            assert!(online.contains_key(key));
        }
    }
}
