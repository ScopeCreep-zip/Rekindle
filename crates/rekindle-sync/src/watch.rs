//! Tracking for records with active Veilid watches.

use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub struct WatchManager {
    watched_records: HashSet<String>,
}

impl WatchManager {
    pub fn mark_active(&mut self, record_key: impl Into<String>) {
        self.watched_records.insert(record_key.into());
    }

    pub fn mark_inactive(&mut self, record_key: &str) {
        self.watched_records.remove(record_key);
    }

    pub fn is_active(&self, record_key: &str) -> bool {
        self.watched_records.contains(record_key)
    }

    pub fn len(&self) -> usize {
        self.watched_records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.watched_records.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::WatchManager;

    #[test]
    fn tracks_active_records() {
        let mut manager = WatchManager::default();
        manager.mark_active("gov");
        manager.mark_active("chan");
        assert!(manager.is_active("gov"));
        manager.mark_inactive("gov");
        assert!(!manager.is_active("gov"));
        assert_eq!(manager.len(), 1);
    }
}
