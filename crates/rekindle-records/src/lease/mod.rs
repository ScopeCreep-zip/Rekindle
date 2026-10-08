//! Lease bookkeeping for DHT records (plan C7.1, D4). Pure: no Veilid.
//!
//! veilid-core keeps **one** open-record entry per record per node, with no
//! refcount: a re-open replaces the writer and the safety selection in
//! place, and a close is node-wide and cancels the record's watch
//! (`storage_manager/open_record.rs:155-169`, `close_record.rs:9-35`). Two
//! parts of the app that each open and close a record therefore close it
//! under each other, and a read-only re-open of a record held writable
//! downgrades it. The table below is the refcount Veilid does not keep: the
//! pool asks it what to do on each acquire and release, and only the pool
//! calls Veilid.
//!
//! - The **writer is sticky.** The first borrower with a writer sets it; a
//!   later borrower never replaces it (a different writer passes its key per
//!   write instead, `SetDHTValueOptions.writer`). So a re-open is only ever
//!   an upgrade from no writer to a writer.
//! - The **watch is the union** of every borrower's subkeys; when the last
//!   watcher goes, the watch is cancelled.
//! - The **last release closes.**

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rekindle_types::id::ChannelId;

/// One borrow of a record. Handed to code that must not see Veilid types
/// (the deps traits), and to the pool, which keeps the Veilid side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeaseId(pub u64);

/// Subkeys, as a set (records here have at most 255).
pub type SubkeySet = BTreeSet<u32>;

/// What the pool must do with Veilid for an acquire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenPlan<W> {
    /// The record is open with a writer at least as strong: nothing to do.
    AlreadyOpen,
    /// First borrower: open it, with this writer.
    Open { writer: Option<W> },
    /// Open without a writer, and this borrower brings one: re-open with
    /// it. The only re-open there is, so the writer never downgrades.
    Upgrade { writer: W },
}

/// What the pool must do with Veilid for a watch change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchPlan {
    /// The watched subkeys are unchanged.
    Unchanged,
    /// Watch exactly these subkeys (replacing the previous watch).
    Watch(SubkeySet),
    /// Nobody watches the record any more: cancel the watch.
    Cancel,
}

/// What the pool must do with Veilid for a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleasePlan {
    /// Other borrowers remain and the watch is unchanged.
    Kept,
    /// It was the last borrow: close the record.
    Closed(String),
    /// Other borrowers remain, but the released one watched subkeys nobody
    /// else does: apply the shrunk watch (or cancel it).
    Rewatch(String, WatchPlan),
}

#[derive(Debug)]
struct Entry<W> {
    holders: BTreeMap<LeaseId, SubkeySet>,
    writer: Option<W>,
}

impl<W> Entry<W> {
    fn watched(&self) -> SubkeySet {
        self.holders.values().flatten().copied().collect()
    }
}

/// Every record the node holds open, with its borrowers.
#[derive(Debug)]
pub struct LeaseTable<W> {
    next: u64,
    by_key: BTreeMap<String, Entry<W>>,
    key_of: BTreeMap<LeaseId, String>,
}

impl<W> Default for LeaseTable<W> {
    fn default() -> Self {
        Self {
            next: 0,
            by_key: BTreeMap::new(),
            key_of: BTreeMap::new(),
        }
    }
}

impl<W: Clone> LeaseTable<W> {
    /// Borrow `key`, with a writer when this borrower writes.
    pub fn acquire(&mut self, key: &str, writer: Option<W>) -> (LeaseId, OpenPlan<W>) {
        let id = LeaseId(self.next);
        self.next += 1;
        self.key_of.insert(id, key.to_string());
        let plan = match self.by_key.get_mut(key) {
            None => {
                self.by_key.insert(
                    key.to_string(),
                    Entry {
                        holders: BTreeMap::from([(id, SubkeySet::new())]),
                        writer: writer.clone(),
                    },
                );
                OpenPlan::Open { writer }
            }
            Some(entry) => {
                entry.holders.insert(id, SubkeySet::new());
                match (&entry.writer, writer) {
                    (None, Some(w)) => {
                        entry.writer = Some(w.clone());
                        OpenPlan::Upgrade { writer: w }
                    }
                    _ => OpenPlan::AlreadyOpen,
                }
            }
        };
        (id, plan)
    }

    /// End a borrow. `Some(key)` when it was the last: close the record.
    pub fn release(&mut self, id: LeaseId) -> Option<String> {
        match self.release_plan(id) {
            ReleasePlan::Closed(key) => Some(key),
            ReleasePlan::Kept | ReleasePlan::Rewatch(..) => None,
        }
    }

    /// End a borrow, with what Veilid must do: close the record when it was
    /// the last, or shrink the watch to what the remaining borrowers watch.
    pub fn release_plan(&mut self, id: LeaseId) -> ReleasePlan {
        let Some(key) = self.key_of.remove(&id) else {
            return ReleasePlan::Kept;
        };
        let Some(entry) = self.by_key.get_mut(&key) else {
            return ReleasePlan::Kept;
        };
        let before = entry.watched();
        entry.holders.remove(&id);
        if entry.holders.is_empty() {
            self.by_key.remove(&key);
            return ReleasePlan::Closed(key);
        }
        let after = entry.watched();
        if after == before {
            ReleasePlan::Kept
        } else if after.is_empty() {
            ReleasePlan::Rewatch(key, WatchPlan::Cancel)
        } else {
            ReleasePlan::Rewatch(key, WatchPlan::Watch(after))
        }
    }

    /// Undo an [`acquire`](Self::acquire) whose Veilid open failed: the
    /// borrow ends, and a failed [`OpenPlan::Upgrade`] leaves the record
    /// without the writer it never got. `Some(key)` as for `release`.
    pub fn abort(&mut self, id: LeaseId, plan: &OpenPlan<W>) -> Option<String> {
        let key = self.key_of.get(&id).cloned();
        let closed = self.release(id);
        if let (OpenPlan::Upgrade { .. }, Some(key)) = (plan, key) {
            if let Some(entry) = self.by_key.get_mut(&key) {
                entry.writer = None;
            }
        }
        closed
    }

    /// Set this borrower's watched subkeys (empty stops its watch).
    pub fn set_watch(&mut self, id: LeaseId, subkeys: SubkeySet) -> WatchPlan {
        let Some(key) = self.key_of.get(&id) else {
            return WatchPlan::Unchanged;
        };
        let Some(entry) = self.by_key.get_mut(key) else {
            return WatchPlan::Unchanged;
        };
        let before = entry.watched();
        entry.holders.insert(id, subkeys);
        let after = entry.watched();
        if after == before {
            WatchPlan::Unchanged
        } else if after.is_empty() {
            WatchPlan::Cancel
        } else {
            WatchPlan::Watch(after)
        }
    }

    /// The record's sticky writer, for a re-open that must not lose it.
    pub fn writer(&self, key: &str) -> Option<&W> {
        self.by_key.get(key)?.writer.as_ref()
    }

    /// The subkeys the record is watched on (empty: not watched).
    #[must_use]
    pub fn watched(&self, key: &str) -> SubkeySet {
        self.by_key.get(key).map(Entry::watched).unwrap_or_default()
    }

    /// The record `id` borrows.
    pub fn key(&self, id: LeaseId) -> Option<&str> {
        self.key_of.get(&id).map(String::as_str)
    }

    /// Whether anyone holds `key`.
    #[must_use]
    pub fn is_held(&self, key: &str) -> bool {
        self.by_key.contains_key(key)
    }

    /// Every held record.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.by_key.keys().map(String::as_str)
    }

    /// Drop every borrow; the keys to close.
    pub fn clear(&mut self) -> Vec<String> {
        self.key_of.clear();
        std::mem::take(&mut self.by_key).into_keys().collect()
    }
}

/// veilid-core 0.5.7 `storage_manager/types/mod.rs:14`: no subkey holds more.
pub const VEILID_MAX_SUBKEY_SIZE: usize = 32_768;
/// veilid-core 0.5.7 `storage_manager/types/mod.rs:16`: a record's total.
pub const VEILID_MAX_RECORD_DATA_SIZE: usize = 1_048_576;

/// A record schema, as far as its size limit depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaShape {
    /// Every subkey: DFLT `o_cnt`, or SMPL `o_cnt + Σ m_cnt`
    /// (`veilid_api/types/dht/schema/smpl.rs:126-139`).
    pub subkey_count: u32,
}

/// The largest value one subkey of `shape` accepts:
/// `min(32768, 1 MiB / subkey_count)` (`storage_manager/schema.rs:59-64` for
/// DFLT, `:100-104` for SMPL). Veilid adds no encryption overhead to it: the
/// cap is plaintext bytes (`record_encryption.rs:23-50`).
#[must_use]
pub const fn max_subkey_bytes(shape: SchemaShape) -> usize {
    if shape.subkey_count == 0 {
        return 0;
    }
    let share = VEILID_MAX_RECORD_DATA_SIZE / shape.subkey_count as usize;
    if share < VEILID_MAX_SUBKEY_SIZE {
        share
    } else {
        VEILID_MAX_SUBKEY_SIZE
    }
}

/// The leases one community session holds: released together when the
/// member leaves or logs out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommunityLeases {
    pub governance: Option<LeaseId>,
    pub registry: Option<LeaseId>,
    pub channels: HashMap<ChannelId, LeaseId>,
    /// Plate Gate segment records (governance and registry per segment).
    pub segments: Vec<LeaseId>,
    /// `GovernanceOverflow` pages this member wrote or follows.
    pub overflow: Vec<LeaseId>,
}

/// Which slot of a [`CommunityLeases`] a held record fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldKind {
    Governance,
    Registry,
    Channel,
    Segment,
    Overflow,
}

/// What [`CommunityLeases::merge`] did: the leases to release, and the
/// records newly held, by kind and key.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Merged {
    pub surplus: Vec<LeaseId>,
    pub added: Vec<(HeldKind, String)>,
}

impl CommunityLeases {
    /// Every lease, for release.
    pub fn all(&self) -> impl Iterator<Item = LeaseId> + '_ {
        self.governance
            .iter()
            .chain(self.registry.iter())
            .chain(self.channels.values())
            .chain(self.segments.iter())
            .chain(self.overflow.iter())
            .copied()
    }

    /// Fold `incoming` into this set. A lease on a record the set already
    /// holds (by `key_of`) is surplus, so repeat hand-overs never
    /// accumulate; so is a lease `key_of` does not know (already released).
    /// The caller releases the surplus.
    pub fn merge(
        &mut self,
        incoming: CommunityLeases,
        key_of: impl Fn(LeaseId) -> Option<String>,
    ) -> Merged {
        let mut held: BTreeSet<String> = self.all().filter_map(&key_of).collect();
        let mut merged = Merged::default();
        let mut take = |lease: LeaseId, kind: HeldKind, merged: &mut Merged| -> bool {
            if let Some(key) = key_of(lease).filter(|k| held.insert(k.clone())) {
                merged.added.push((kind, key));
                true
            } else {
                merged.surplus.push(lease);
                false
            }
        };
        if let Some(lease) = incoming.governance {
            if take(lease, HeldKind::Governance, &mut merged) {
                self.governance = Some(lease);
            }
        }
        if let Some(lease) = incoming.registry {
            if take(lease, HeldKind::Registry, &mut merged) {
                self.registry = Some(lease);
            }
        }
        for (channel, lease) in incoming.channels {
            if take(lease, HeldKind::Channel, &mut merged) {
                self.channels.insert(channel, lease);
            }
        }
        for lease in incoming.segments {
            if take(lease, HeldKind::Segment, &mut merged) {
                self.segments.push(lease);
            }
        }
        for lease in incoming.overflow {
            if take(lease, HeldKind::Overflow, &mut merged) {
                self.overflow.push(lease);
            }
        }
        merged
    }
}

#[cfg(test)]
mod tests;
