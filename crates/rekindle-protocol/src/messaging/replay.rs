//! Receive-side replay guard for 1:1 envelopes.
//!
//! An envelope is accepted once: its `(sender, nonce)` is remembered until
//! its timestamp leaves the freshness window, after which
//! [`crate::messaging::receiver::process_incoming`] rejects it as stale
//! anyway. A retry keeps the nonce and re-signs with a fresh timestamp, so
//! a copy of a message that already arrived is dropped as a duplicate,
//! while one that never arrived goes through.

use std::collections::{HashMap, VecDeque};

use blake2::{digest::consts::U16, Blake2b, Digest};
use rekindle_types::message::{ENVELOPE_FRESHNESS_WINDOW_MS, ENVELOPE_MAX_FUTURE_SKEW_MS};

/// Most `(sender, nonce)` pairs held at once. Entries leave when they
/// expire; a flood of fresh envelopes beyond this is refused rather than
/// evicting live entries, which would reopen the replay window.
pub const MAX_REMEMBERED: usize = 65_536;

type ReplayKey = [u8; 16];

/// What the guard decided about one envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sighting {
    /// Not seen before; now remembered.
    First,
    /// Already accepted within the freshness window.
    Duplicate,
    /// The guard is full of unexpired entries.
    Full,
}

/// Remembers accepted envelopes for as long as they could still verify.
#[derive(Debug, Default)]
pub struct ReplayGuard {
    expiry_by_key: HashMap<ReplayKey, u64>,
    /// Keys in insertion order with their expiry, for pruning.
    order: VecDeque<(ReplayKey, u64)>,
}

impl ReplayGuard {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the envelope `(sender, nonce)` signed at `timestamp`, as seen
    /// at `now_ms`. Call only after the signature and freshness checks.
    pub fn observe(
        &mut self,
        sender: &[u8],
        nonce: &[u8],
        timestamp: u64,
        now_ms: u64,
    ) -> Sighting {
        self.prune(now_ms);
        let key = replay_key(sender, nonce);
        if self.expiry_by_key.contains_key(&key) {
            return Sighting::Duplicate;
        }
        if self.expiry_by_key.len() >= MAX_REMEMBERED {
            return Sighting::Full;
        }
        // Valid until the timestamp is too old to pass the freshness check;
        // a timestamp ahead of our clock stays valid that much longer.
        let expiry = timestamp
            .max(now_ms)
            .saturating_add(ENVELOPE_FRESHNESS_WINDOW_MS + ENVELOPE_MAX_FUTURE_SKEW_MS);
        self.expiry_by_key.insert(key, expiry);
        self.order.push_back((key, expiry));
        Sighting::First
    }

    /// Forget everything (logout).
    pub fn clear(&mut self) {
        self.expiry_by_key.clear();
        self.order.clear();
    }

    fn prune(&mut self, now_ms: u64) {
        // Expiries are not strictly ordered (future-skewed timestamps), so
        // walk the queue while the front has expired and requeue nothing:
        // a live entry at the front stops the walk, and its successors are
        // pruned on a later call once it expires.
        while let Some(&(key, expiry)) = self.order.front() {
            if expiry > now_ms {
                break;
            }
            self.order.pop_front();
            self.expiry_by_key.remove(&key);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.expiry_by_key.len()
    }
}

fn replay_key(sender: &[u8], nonce: &[u8]) -> ReplayKey {
    let mut hasher = Blake2b::<U16>::new();
    hasher.update(
        u32::try_from(sender.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    hasher.update(sender);
    hasher.update(nonce);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000_000;

    #[test]
    fn first_then_duplicate() {
        let mut guard = ReplayGuard::new();
        assert_eq!(guard.observe(b"alice", b"n1", NOW, NOW), Sighting::First);
        assert_eq!(
            guard.observe(b"alice", b"n1", NOW + 5, NOW + 10),
            Sighting::Duplicate
        );
        assert_eq!(guard.observe(b"alice", b"n2", NOW, NOW), Sighting::First);
        assert_eq!(guard.observe(b"bob", b"n1", NOW, NOW), Sighting::First);
    }

    #[test]
    fn entries_expire_with_the_freshness_window() {
        let mut guard = ReplayGuard::new();
        guard.observe(b"alice", b"n1", NOW, NOW);
        let after = NOW + ENVELOPE_FRESHNESS_WINDOW_MS + ENVELOPE_MAX_FUTURE_SKEW_MS + 1;
        guard.observe(b"carol", b"x", after, after);
        assert_eq!(guard.len(), 1, "the expired entry is pruned");
    }

    #[test]
    fn full_guard_refuses_instead_of_evicting() {
        let mut guard = ReplayGuard::new();
        for i in 0..MAX_REMEMBERED {
            let nonce = u32::try_from(i).unwrap().to_le_bytes();
            assert_eq!(guard.observe(b"s", &nonce, NOW, NOW), Sighting::First);
        }
        assert_eq!(guard.observe(b"s", b"new", NOW, NOW), Sighting::Full);
        assert_eq!(
            guard.observe(b"s", &0u32.to_le_bytes(), NOW, NOW),
            Sighting::Duplicate,
            "live entries are never evicted early"
        );
    }
}
