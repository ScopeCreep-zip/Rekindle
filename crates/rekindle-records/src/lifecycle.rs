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
//! ## Why this shape
//!
//! This struct used to model `channel_keys` as `HashMap<channel_id,
//! record_key>` and carry nothing else, while src-tauri kept a second
//! `CommunityRecords` that had grown the fields the lifecycle actually
//! needs — a writer keypair to re-open the registry without clobbering
//! it, the GovernanceOverflow chain, an open/closed flag, and the
//! inspect fingerprints the sync loop compares. The host's version was
//! the one that met reality, so it is the one that survived; this crate
//! keeps ownership because "DHT record lifecycle" is its stated job.
//!
//! `channel_keys` is a flat `Vec` rather than a channel map because
//! that is what it holds: every record key open for the community,
//! including the per-segment governance and registry keys a Plate Gate
//! expansion adds. Keying it by channel id could not represent those.

use std::collections::HashMap;

/// The record keys of a single community session.
///
/// Created on community join/create, dropped on leave/logout. The records
/// themselves are held, and closed, through the community's leases in the
/// record pool.
#[derive(Debug, Clone, Default)]
pub struct CommunityRecords {
    /// Governance SMPL record (`o_cnt:0`) — community state.
    pub governance_key: Option<String>,

    /// Member registry SMPL record (`o_cnt:0`) — presence + route blobs.
    pub registry_key: Option<String>,

    /// Every channel/segment SMPL record key open for this community.
    pub channel_keys: Vec<String>,

    /// Member-owned `GovernanceOverflow` record keys — the author's spill
    /// pages, and any overflow chain this node has *followed* as a reader
    /// (Mutual Aid §14.1: readers keep what they read alive). Warmed,
    /// tracked open, and closed on leave like `channel_keys`; never
    /// *watched*, because the primary subkey's watch already re-follows
    /// the chain on change.
    pub governance_overflow_keys: Vec<String>,

    /// Fingerprint of the last inspected governance record state.
    pub governance_report_fingerprint: Option<u64>,

    /// Fingerprints of the last inspected channel record state, by
    /// channel id.
    pub channel_report_fingerprints: HashMap<String, u64>,
}

impl CommunityRecords {
    /// Create a new tracker for a community.
    #[must_use]
    pub fn new(governance_key: String, registry_key: String) -> Self {
        Self {
            governance_key: Some(governance_key),
            registry_key: Some(registry_key),
            ..Self::default()
        }
    }

    /// Track a channel/segment record, ignoring a repeat.
    pub fn add_channel(&mut self, record_key: String) {
        if !self.channel_keys.contains(&record_key) {
            self.channel_keys.push(record_key);
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
        if let Some(ref k) = self.governance_key {
            keys.push(k.clone());
        }
        if let Some(ref k) = self.registry_key {
            keys.push(k.clone());
        }
        keys.extend(self.channel_keys.iter().cloned());
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
mod tests {
    use super::*;

    #[test]
    fn all_keys_includes_the_overflow_chain() {
        let mut r = CommunityRecords::new("gov".into(), "reg".into());
        r.add_channel("ch1".into());
        r.add_channel("ch1".into()); // repeat ignored
        r.add_overflow("of1".into());
        assert_eq!(r.all_keys(), vec!["gov", "reg", "ch1", "of1"]);
        assert_eq!(r.record_count(), 4);
    }
}
