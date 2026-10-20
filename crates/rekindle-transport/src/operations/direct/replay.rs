//! Receive-side replay guard for pre-session frames (plan E2.3; moved
//! from `rekindle-protocol`).
//!
//! Ratchet frames need none: a used `(dh, n)` has no key left. A sealed
//! frame is accepted once: its `(sender, nonce)` is remembered under
//! `rekindle_crypto::replay::replay_id` until its `sent_at` leaves the
//! freshness window, after which the frame is refused as stale anyway. A
//! retry keeps the nonce and re-seals with a fresh `sent_at`, so a copy of
//! a frame that already arrived is dropped as a duplicate, while one that
//! never arrived goes through.
//!
//! Every sender has its own share (plan audit G7): a sender holding
//! [`PER_SENDER_QUOTA`] live entries is refused, and so is one spending
//! opens faster than its bucket refills ([`ReplayGuard::may_open`], checked
//! after the signature and before the HPKE open), so one sender filling its
//! share blocks no one else. [`MAX_REMEMBERED`] is the backstop over all
//! senders together: it is reached only when that many distinct senders
//! each hold entries, and live entries are never evicted to make room,
//! which would reopen the replay window.

use std::collections::{HashMap, VecDeque};

use rekindle_crypto::replay::replay_id;
use rekindle_types::message::{ENVELOPE_FRESHNESS_WINDOW_MS, ENVELOPE_MAX_FUTURE_SKEW_MS};

/// Most `(sender, nonce)` pairs held at once, over every sender.
pub const MAX_REMEMBERED: usize = 65_536;

/// Most live entries one sender may hold. Sealed kinds are rare (a
/// request, a reject, a reset, a push registration or wake), so a friend
/// never comes near it inside one freshness window.
pub const PER_SENDER_QUOTA: usize = 256;

/// A sender's open bucket: its burst, and one more open per this many
/// milliseconds.
const BUCKET_BURST: u32 = 32;
const BUCKET_REFILL_MS: u64 = 1_000;

/// Most senders with a partly spent bucket tracked at once; a full bucket
/// is the same as none, so idle senders are dropped first.
const MAX_BUCKETS: usize = 4_096;

type ReplayKey = [u8; 32];
type Sender = [u8; 32];

/// What the guard decided about one envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sighting {
    /// Not seen before; now remembered.
    First,
    /// Already accepted within the freshness window.
    Duplicate,
    /// This sender holds its quota of unexpired entries (or every sender
    /// together fills the backstop).
    Full,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: u32,
    at_ms: u64,
}

impl Bucket {
    fn refilled(self, now_ms: u64) -> Self {
        let earned = now_ms.saturating_sub(self.at_ms) / BUCKET_REFILL_MS;
        let earned = u32::try_from(earned).unwrap_or(u32::MAX);
        Self {
            tokens: self.tokens.saturating_add(earned).min(BUCKET_BURST),
            at_ms: self
                .at_ms
                .saturating_add(u64::from(earned).saturating_mul(BUCKET_REFILL_MS)),
        }
    }
}

/// Remembers accepted envelopes for as long as they could still verify.
#[derive(Debug, Default)]
pub struct ReplayGuard {
    expiry_by_key: HashMap<ReplayKey, u64>,
    /// Keys in insertion order with their sender and expiry, for pruning.
    order: VecDeque<(ReplayKey, Sender, u64)>,
    /// Live entries per sender.
    held: HashMap<Sender, usize>,
    buckets: HashMap<Sender, Bucket>,
}

impl ReplayGuard {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `sender` (whose signature verified) may spend an HPKE open
    /// at `now_ms`: its bucket has a token left. Spends it.
    pub fn may_open(&mut self, sender: &Sender, now_ms: u64) -> bool {
        if !self.buckets.contains_key(sender) && self.buckets.len() >= MAX_BUCKETS {
            self.buckets
                .retain(|_, bucket| bucket.refilled(now_ms).tokens < BUCKET_BURST);
            if self.buckets.len() >= MAX_BUCKETS {
                return false;
            }
        }
        let bucket = self.buckets.entry(*sender).or_insert(Bucket {
            tokens: BUCKET_BURST,
            at_ms: now_ms,
        });
        let mut refilled = bucket.refilled(now_ms);
        let admitted = refilled.tokens > 0;
        if admitted {
            refilled.tokens -= 1;
        }
        *bucket = refilled;
        admitted
    }

    /// Record the envelope `(sender, nonce)` signed at `timestamp`, as seen
    /// at `now_ms`. Call only after the signature and freshness checks.
    pub fn observe(
        &mut self,
        sender: &Sender,
        nonce: &[u8; 16],
        timestamp: u64,
        now_ms: u64,
    ) -> Sighting {
        self.prune(now_ms);
        let key = replay_id(sender, nonce);
        if self.expiry_by_key.contains_key(&key) {
            return Sighting::Duplicate;
        }
        let held = self.held.get(sender).copied().unwrap_or(0);
        if held >= PER_SENDER_QUOTA || self.expiry_by_key.len() >= MAX_REMEMBERED {
            return Sighting::Full;
        }
        // Valid until the timestamp is too old to pass the freshness check;
        // a timestamp ahead of our clock stays valid that much longer.
        let expiry = timestamp
            .max(now_ms)
            .saturating_add(ENVELOPE_FRESHNESS_WINDOW_MS + ENVELOPE_MAX_FUTURE_SKEW_MS);
        self.expiry_by_key.insert(key, expiry);
        self.order.push_back((key, *sender, expiry));
        *self.held.entry(*sender).or_insert(0) += 1;
        Sighting::First
    }

    /// Forget `(sender, nonce)`: it opened but was not applied, so a retry
    /// under the same nonce must go through.
    pub fn forget(&mut self, sender: &Sender, nonce: &[u8; 16]) {
        let key = replay_id(sender, nonce);
        if self.expiry_by_key.remove(&key).is_some() {
            self.order.retain(|(k, _, _)| *k != key);
            self.release(sender);
        }
    }

    /// Forget everything (logout).
    pub fn clear(&mut self) {
        self.expiry_by_key.clear();
        self.order.clear();
        self.held.clear();
        self.buckets.clear();
    }

    fn release(&mut self, sender: &Sender) {
        if let Some(held) = self.held.get_mut(sender) {
            *held = held.saturating_sub(1);
            if *held == 0 {
                self.held.remove(sender);
            }
        }
    }

    fn prune(&mut self, now_ms: u64) {
        // Expiries are not strictly ordered (future-skewed timestamps), so
        // walk the queue while the front has expired and requeue nothing:
        // a live entry at the front stops the walk, and its successors are
        // pruned on a later call once it expires.
        while let Some(&(key, sender, expiry)) = self.order.front() {
            if expiry > now_ms {
                break;
            }
            self.order.pop_front();
            self.expiry_by_key.remove(&key);
            self.release(&sender);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.expiry_by_key.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000_000;
    const ALICE: Sender = [1; 32];
    const BOB: Sender = [2; 32];

    fn nonce(i: usize) -> [u8; 16] {
        let mut n = [0u8; 16];
        n[..8].copy_from_slice(&u64::try_from(i).unwrap().to_le_bytes());
        n
    }

    #[test]
    fn first_then_duplicate() {
        let mut guard = ReplayGuard::new();
        assert_eq!(guard.observe(&ALICE, &nonce(1), NOW, NOW), Sighting::First);
        assert_eq!(
            guard.observe(&ALICE, &nonce(1), NOW + 5, NOW + 10),
            Sighting::Duplicate
        );
        assert_eq!(guard.observe(&ALICE, &nonce(2), NOW, NOW), Sighting::First);
        assert_eq!(guard.observe(&BOB, &nonce(1), NOW, NOW), Sighting::First);
    }

    #[test]
    fn entries_expire_with_the_freshness_window() {
        let mut guard = ReplayGuard::new();
        guard.observe(&ALICE, &nonce(1), NOW, NOW);
        let after = NOW + ENVELOPE_FRESHNESS_WINDOW_MS + ENVELOPE_MAX_FUTURE_SKEW_MS + 1;
        guard.observe(&BOB, &nonce(1), after, after);
        assert_eq!(guard.len(), 1, "the expired entry is pruned");
        assert!(!guard.held.contains_key(&ALICE), "its share is released");
    }

    #[test]
    fn a_sender_over_its_quota_blocks_no_one_else() {
        let mut guard = ReplayGuard::new();
        for i in 0..PER_SENDER_QUOTA {
            assert_eq!(guard.observe(&ALICE, &nonce(i), NOW, NOW), Sighting::First);
        }
        assert_eq!(
            guard.observe(&ALICE, &nonce(PER_SENDER_QUOTA), NOW, NOW),
            Sighting::Full
        );
        assert_eq!(
            guard.observe(&ALICE, &nonce(0), NOW, NOW),
            Sighting::Duplicate,
            "live entries are never evicted early"
        );
        assert_eq!(guard.observe(&BOB, &nonce(0), NOW, NOW), Sighting::First);
        // A forgotten entry gives its share back.
        guard.forget(&ALICE, &nonce(0));
        assert_eq!(
            guard.observe(&ALICE, &nonce(PER_SENDER_QUOTA), NOW, NOW),
            Sighting::First
        );
    }

    #[test]
    fn a_sender_spending_its_bucket_leaves_others_theirs() {
        let mut guard = ReplayGuard::new();
        for _ in 0..BUCKET_BURST {
            assert!(guard.may_open(&ALICE, NOW));
        }
        assert!(!guard.may_open(&ALICE, NOW));
        assert!(guard.may_open(&BOB, NOW));
        assert!(guard.may_open(&ALICE, NOW + BUCKET_REFILL_MS), "it refills");
    }
}
