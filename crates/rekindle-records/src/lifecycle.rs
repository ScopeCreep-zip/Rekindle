//! DHT record lifecycle tracking.
//!
//! Addresses Gap G from rekindle-architecture-v2.md §15.
//!
//! Records are opened once on community join and kept open for the entire
//! session. This avoids the overhead of DHT routing table lookups on every
//! read/write. The `CommunityRecords` struct tracks all open records per
//! community so they can be properly closed on leave/logout.
//!
//! See architecture doc §4.8 and VeilidChat pattern.

use std::collections::HashMap;

/// Tracks all DHT records open for a single community session.
///
/// Created on community join/create, destroyed on leave/logout.
/// All records should be closed via `close_dht_record()` on teardown.
#[derive(Debug, Clone, Default)]
pub struct CommunityRecords {
    /// Governance SMPL record (o_cnt:0) — community state.
    pub governance_key: Option<String>,

    /// Member registry SMPL record (o_cnt:0) — presence + route blobs.
    pub registry_key: Option<String>,

    /// Channel SMPL records — one per active channel.
    /// Key: channel_id (hex or UUID string), Value: DHT record key string.
    pub channel_keys: HashMap<String, String>,

    /// Owner keypair string for reopening records on restart.
    /// Even with o_cnt:0, the owner keypair is needed to call open_dht_record.
    pub owner_keypair: Option<String>,
}

impl CommunityRecords {
    /// Create a new tracker for a community.
    pub fn new(governance_key: String, registry_key: String) -> Self {
        Self {
            governance_key: Some(governance_key),
            registry_key: Some(registry_key),
            channel_keys: HashMap::new(),
            owner_keypair: None,
        }
    }

    /// Track a channel record.
    pub fn add_channel(&mut self, channel_id: String, record_key: String) {
        self.channel_keys.insert(channel_id, record_key);
    }

    /// Stop tracking a channel record (on channel archive).
    pub fn remove_channel(&mut self, channel_id: &str) -> Option<String> {
        self.channel_keys.remove(channel_id)
    }

    /// Get all tracked record keys (for bulk close on leave).
    pub fn all_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        if let Some(ref k) = self.governance_key {
            keys.push(k.clone());
        }
        if let Some(ref k) = self.registry_key {
            keys.push(k.clone());
        }
        keys.extend(self.channel_keys.values().cloned());
        keys
    }

    /// Number of tracked records.
    pub fn record_count(&self) -> usize {
        let mut count = 0;
        if self.governance_key.is_some() {
            count += 1;
        }
        if self.registry_key.is_some() {
            count += 1;
        }
        count += self.channel_keys.len();
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tracks_governance_and_registry() {
        let records = CommunityRecords::new("gov_key".into(), "reg_key".into());
        assert_eq!(records.governance_key.as_deref(), Some("gov_key"));
        assert_eq!(records.registry_key.as_deref(), Some("reg_key"));
        assert_eq!(records.record_count(), 2);
    }

    #[test]
    fn add_and_remove_channel() {
        let mut records = CommunityRecords::new("g".into(), "r".into());
        records.add_channel("ch1".into(), "ch1_key".into());
        records.add_channel("ch2".into(), "ch2_key".into());
        assert_eq!(records.record_count(), 4); // gov + reg + 2 channels

        let removed = records.remove_channel("ch1");
        assert_eq!(removed, Some("ch1_key".into()));
        assert_eq!(records.record_count(), 3);
    }

    #[test]
    fn all_keys_returns_everything() {
        let mut records = CommunityRecords::new("g".into(), "r".into());
        records.add_channel("ch1".into(), "ck1".into());
        let keys = records.all_keys();
        assert_eq!(keys.len(), 3);
        assert!(keys.contains(&"g".to_string()));
        assert!(keys.contains(&"r".to_string()));
        assert!(keys.contains(&"ck1".to_string()));
    }
}
