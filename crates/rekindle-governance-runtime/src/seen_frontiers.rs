//! What a member has read of one community's member-record frontiers
//! (`M`/3), shared by every path that reads them (review S7m part 2 P2,
//! part 3; `evidence/s7m-items.md` 2c + 4-rest): the periodic sweep, the
//! login sweep, the watch ring's changes and the `HeadAdvanced` hints all
//! hold one [`SeenFrontiers`] per community (the node keeps it in the
//! community's `HeadHints`), so a frontier one path read is not read and
//! merged again by another.
//!
//! It also bounds D15's convergence check: a peer whose equal heads carry
//! another roster checksum is a divergence signal, warned once per
//! (record, heads, checksum) and never a rebuild (rebuilding the same
//! deterministic merge of the same op set cannot change our checksum); our
//! own state is rebuilt only when the installed checksum lags the merge of
//! our op set, once per op set. The merge itself is cached by op-set
//! digest, so a repeated check costs no full merge.
//!
//! In memory only: the login sweep reads every frontier again after a
//! restart.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_types::governance::{OpHash, RosterChecksum};

/// A digest of an op set (the sorted hashes of the ops held), the key of
/// the cached merge. In-memory only, never on the wire.
pub type OpSetDigest = [u8; 32];

/// The heads and roster checksum of one merge of an op set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedView {
    pub heads: BTreeSet<OpHash>,
    pub checksum: RosterChecksum,
}

/// One community's frontier reads, convergence memo and cached merge; a
/// clone shares them.
#[derive(Debug, Clone, Default)]
pub struct SeenFrontiers(Arc<Mutex<Seen>>);

#[derive(Debug, Default)]
struct Seen {
    /// The frontier seq read per member record.
    read: BTreeMap<String, u64>,
    /// The minimum a trigger named during each active catch-up.
    minimum: BTreeMap<String, u64>,
    /// The (heads digest, checksum) last warned about per member record.
    divergent: BTreeMap<String, (OpSetDigest, RosterChecksum)>,
    /// Our merge of the op set with this digest.
    merged: Option<(OpSetDigest, MergedView)>,
    /// The op set whose lagging installed state was last rebuilt.
    lag_rebuilt: Option<OpSetDigest>,
}

/// The digest of the op set whose hashes are `hashes`, in any order.
#[must_use]
pub fn op_set_digest<'a>(hashes: impl IntoIterator<Item = &'a OpHash>) -> OpSetDigest {
    let sorted: BTreeSet<&OpHash> = hashes.into_iter().collect();
    let mut hasher = blake3::Hasher::new();
    for hash in sorted {
        hasher.update(&hash.0);
    }
    *hasher.finalize().as_bytes()
}

impl SeenFrontiers {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The frontier seq read of `record`.
    #[must_use]
    pub fn get(&self, record: &str) -> Option<u64> {
        self.0.lock().read.get(record).copied()
    }

    /// Is `record`'s frontier at `seq` unread (above the seq read, or
    /// never read; a frontier's first value is at seq 0)?
    #[must_use]
    pub fn unread(&self, record: &str, seq: u64) -> bool {
        self.get(record).is_none_or(|read| seq > read)
    }

    /// Set the frontier floor while a trigger is being serviced. The
    /// sweeps share the read marker and must also respect this floor.
    pub fn set_minimum(&self, record: &str, minimum: Option<u64>) {
        let mut seen = self.0.lock();
        if let Some(minimum) = minimum {
            seen.minimum.insert(record.to_owned(), minimum);
        } else {
            seen.minimum.remove(record);
        }
    }

    /// Mark `record`'s frontier read at `seq` when it reaches the active
    /// trigger's minimum. Returns true when it advances the read marker.
    pub fn mark_read(&self, record: &str, seq: u64) -> bool {
        use std::collections::btree_map::Entry;
        let mut seen = self.0.lock();
        if seen
            .minimum
            .get(record)
            .is_some_and(|minimum| seq < *minimum)
        {
            return false;
        }
        match seen.read.entry(record.to_owned()) {
            Entry::Vacant(slot) => {
                slot.insert(seq);
                true
            }
            Entry::Occupied(mut slot) => {
                let moved = seq > *slot.get();
                *slot.get_mut() = (*slot.get()).max(seq);
                moved
            }
        }
    }

    /// Note that `record`'s frontier with heads `heads` carries `checksum`,
    /// not ours: `true` the first time this (heads, checksum) is noted for
    /// the record, so the caller warns once.
    pub fn note_divergence(
        &self,
        record: &str,
        heads: OpSetDigest,
        checksum: RosterChecksum,
    ) -> bool {
        let mut seen = self.0.lock();
        let tuple = (heads, checksum);
        if seen.divergent.get(record) == Some(&tuple) {
            return false;
        }
        seen.divergent.insert(record.to_owned(), tuple);
        true
    }

    /// Our cached merge of the op set with `digest`.
    #[must_use]
    pub fn merged(&self, digest: &OpSetDigest) -> Option<MergedView> {
        self.0
            .lock()
            .merged
            .as_ref()
            .filter(|(cached, _)| cached == digest)
            .map(|(_, view)| view.clone())
    }

    /// Cache our merge of the op set with `digest`, replacing the last.
    pub fn cache_merged(&self, digest: OpSetDigest, view: MergedView) {
        self.0.lock().merged = Some((digest, view));
    }

    /// Claim the rebuild of an installed state that lags the op set with
    /// `digest`: `false` when that op set was already rebuilt (a lag the
    /// rebuild did not close is not rebuilt again, the caller warns).
    pub fn claim_lag_rebuild(&self, digest: OpSetDigest) -> bool {
        let mut seen = self.0.lock();
        if seen.lag_rebuilt == Some(digest) {
            return false;
        }
        seen.lag_rebuilt = Some(digest);
        true
    }

    /// Forget every record not in `records` (the current members'
    /// records): a departed member's entries are never read again.
    pub fn retain_records(&self, records: &BTreeSet<String>) {
        let mut seen = self.0.lock();
        seen.read.retain(|record, _| records.contains(record));
        seen.divergent.retain(|record, _| records.contains(record));
    }
}

#[cfg(test)]
#[path = "seen_frontiers_tests.rs"]
mod tests;
