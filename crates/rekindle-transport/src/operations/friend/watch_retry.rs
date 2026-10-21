//! Failed watch opens (plan E2.6): a friend's profile record, a friend's
//! participant record or a member's group chain can fail to open just
//! after it is created (it is not reachable yet). The failure is kept here,
//! per record, until the open succeeds, the watch is dropped, or we lock;
//! the friend maintenance tick re-opens what is due, with doubling backoff,
//! a cap and equal jitter from the Host's generator. Only the watch is
//! re-opened (nothing polls values); the Host diffs once it opens, for what
//! changed while no watch of ours was open.

use std::collections::HashMap;

use rekindle_utils::RandomSource;

/// The wait before the first retry, the friend maintenance's tick.
pub const FIRST_DELAY_MS: u64 = 30_000;
/// The longest wait between two retries.
pub const MAX_DELAY_MS: u64 = 15 * 60_000;

/// What a failed open was for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchTarget {
    /// A friend's profile record.
    Profile { peer: String },
    /// A participant record: a friend's DM chain to us (`key` is the
    /// friend) or a member's group chain (`group_dm::receive::watch_key`).
    Participant { key: String },
}

impl WatchTarget {
    fn is_of(&self, peer: &str) -> bool {
        match self {
            Self::Profile { peer: p } => p == peer,
            Self::Participant { key } => key == peer,
        }
    }
}

struct Pending {
    target: WatchTarget,
    attempts: u32,
    due_ms: u64,
}

/// The failed opens, by record key.
#[derive(Default)]
pub struct WatchRetries {
    pending: HashMap<String, Pending>,
}

impl WatchRetries {
    /// Opening `record` for `target` failed at `now_ms`: its next attempt
    /// waits one more backoff step.
    pub fn failed(
        &mut self,
        record: &str,
        target: WatchTarget,
        now_ms: u64,
        rng: &dyn RandomSource,
    ) {
        let attempts = self
            .pending
            .get(record)
            .map_or(0, |p| p.attempts)
            .saturating_add(1);
        let due_ms = now_ms.saturating_add(backoff(attempts, rng));
        self.pending.insert(
            record.to_string(),
            Pending {
                target,
                attempts,
                due_ms,
            },
        );
    }

    /// `record` is open, or no longer wanted: nothing to retry.
    pub fn settled(&mut self, record: &str) {
        self.pending.remove(record);
    }

    /// `peer`'s watches are dropped (unwatch): forget their failures.
    pub fn forget(&mut self, peer: &str) {
        self.pending.retain(|_, p| !p.target.is_of(peer));
    }

    /// Whether `record` waits for a retry.
    #[must_use]
    pub fn is_pending(&self, record: &str) -> bool {
        self.pending.contains_key(record)
    }

    /// The records whose retry is due at `now_ms`, with their targets.
    #[must_use]
    pub fn due(&self, now_ms: u64) -> Vec<(String, WatchTarget)> {
        self.pending
            .iter()
            .filter(|(_, p)| p.due_ms <= now_ms)
            .map(|(record, p)| (record.clone(), p.target.clone()))
            .collect()
    }

    /// Forget everything (lock).
    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

/// The wait after the `attempts`-th failure: [`FIRST_DELAY_MS`] doubled per
/// earlier failure, capped at [`MAX_DELAY_MS`], half of it fixed and half
/// drawn from `rng` (equal jitter), so members who failed together do not
/// retry together.
#[must_use]
pub fn backoff(attempts: u32, rng: &dyn RandomSource) -> u64 {
    let doublings = attempts.saturating_sub(1).min(16);
    let step = FIRST_DELAY_MS
        .saturating_mul(1u64 << doublings)
        .min(MAX_DELAY_MS);
    step / 2 + rng.below(step / 2 + 1)
}

#[cfg(test)]
mod tests {
    use rekindle_utils::OsRandom;

    use super::*;

    fn profile(peer: &str) -> WatchTarget {
        WatchTarget::Profile {
            peer: peer.to_string(),
        }
    }

    #[test]
    fn the_backoff_doubles_with_jitter_up_to_the_cap() {
        let rng = OsRandom;
        for attempts in 1..40 {
            let step = (FIRST_DELAY_MS << (attempts - 1).min(16)).min(MAX_DELAY_MS);
            let wait = backoff(attempts, &rng);
            assert!(
                wait >= step / 2 && wait <= step,
                "attempt {attempts}: {wait}"
            );
        }
    }

    #[test]
    fn a_failure_is_due_after_its_backoff_until_it_settles() {
        let rng = OsRandom;
        let mut retries = WatchRetries::default();
        retries.failed("rec", profile("ann"), 1_000, &rng);
        assert!(retries.due(1_000 + FIRST_DELAY_MS / 2 - 1).is_empty());
        assert_eq!(
            retries.due(1_000 + FIRST_DELAY_MS),
            vec![("rec".to_string(), profile("ann"))]
        );
        retries.failed("rec", profile("ann"), 50_000, &rng);
        assert!(retries.due(50_000 + FIRST_DELAY_MS - 1).is_empty());
        assert_eq!(retries.due(50_000 + 2 * FIRST_DELAY_MS).len(), 1);
        retries.settled("rec");
        assert!(!retries.is_pending("rec"));
    }

    #[test]
    fn an_unwatch_and_a_lock_forget_the_failures() {
        let rng = OsRandom;
        let mut retries = WatchRetries::default();
        retries.failed("p", profile("ann"), 0, &rng);
        let dm = WatchTarget::Participant {
            key: "ann".to_string(),
        };
        retries.failed("d", dm, 0, &rng);
        let group = WatchTarget::Participant {
            key: "g:bea".to_string(),
        };
        retries.failed("g", group, 0, &rng);
        retries.forget("ann");
        assert!(!retries.is_pending("p") && !retries.is_pending("d"));
        assert!(retries.is_pending("g"));
        retries.clear();
        assert!(!retries.is_pending("g"));
    }
}
