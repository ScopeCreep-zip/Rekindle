//! DHT record lifecycle tracking.
//!
//! Addresses Gap G from rekindle-architecture-v2.md §15.
//!
//! Records are opened once on community join and kept open for the entire
//! session. The `CommunityRecords` struct is the community's record-key
//! inventory (and the inspect fingerprints); whether a record is open, and
//! with which writer, is the record pool's `CommunityLeases` (plan C7.5).
//!
//! See architecture doc §4.8 and VeilidChat pattern.
//!
//! `segment_keys` holds the per-segment governance and registry keys a
//! Plate Gate expansion adds (plan E3.4); a channel has no record (plan
//! E3.3), so none is tracked here.

/// The record keys of a single community session.
///
/// Created on community join/create, dropped on leave/logout. The records
/// themselves are held, and closed, through the community's leases in the
/// record pool.
#[derive(Debug, Clone, Default)]
pub struct CommunityRecords {
    /// Governance SMPL record (`o_cnt:0`) — community state.
    pub anchor: Option<String>,

    /// Member registry SMPL record (`o_cnt:0`) — presence + route blobs.
    pub registry_key: Option<String>,

    /// Every Plate Gate segment SMPL record key (registry and governance
    /// per segment, plan E3.4) open for this community. A channel has no
    /// record (plan E3.3).
    pub segment_keys: Vec<String>,

    /// Member-owned `GovernanceOverflow` record keys — the author's spill
    /// pages, and any overflow chain this node has *followed* as a reader
    /// (Mutual Aid §14.1: readers keep what they read alive). Warmed,
    /// tracked open, and closed on leave like `segment_keys`; never
    /// *watched*, because the primary subkey's watch already re-follows
    /// the chain on change.
    pub governance_overflow_keys: Vec<String>,
}

impl CommunityRecords {
    /// Create a new tracker for a community.
    #[must_use]
    pub fn new(anchor: String, registry_key: String) -> Self {
        Self {
            anchor: Some(anchor),
            registry_key: Some(registry_key),
            ..Self::default()
        }
    }

    /// Track a segment record, ignoring a repeat.
    pub fn add_segment(&mut self, record_key: String) {
        if !self.segment_keys.contains(&record_key) {
            self.segment_keys.push(record_key);
        }
    }

    /// Track a `GovernanceOverflow` record, ignoring a repeat.
    pub fn add_overflow(&mut self, record_key: String) {
        if !self.governance_overflow_keys.contains(&record_key) {
            self.governance_overflow_keys.push(record_key);
        }
    }

    /// Every tracked record key, for bulk close on leave.
    #[must_use]
    pub fn all_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        if let Some(ref k) = self.anchor {
            keys.push(k.clone());
        }
        if let Some(ref k) = self.registry_key {
            keys.push(k.clone());
        }
        keys.extend(self.segment_keys.iter().cloned());
        keys.extend(self.governance_overflow_keys.iter().cloned());
        keys
    }

    /// Number of tracked records.
    #[must_use]
    pub fn record_count(&self) -> usize {
        self.all_keys().len()
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
