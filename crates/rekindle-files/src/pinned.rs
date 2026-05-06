//! Pinned-attachment set — chunks for these IDs are exempt from LRU
//! eviction (spec §28.9 line 3283). The set is driven by
//! `GovernanceEntry::AttachmentPinned` merge state; this struct is just
//! the in-memory mirror the cache reads on every eviction sweep.

use std::collections::HashSet;

use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct PinnedSet {
    ids: HashSet<Uuid>,
}

impl PinnedSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, id: Uuid) {
        self.ids.insert(id);
    }

    pub fn remove(&mut self, id: &Uuid) {
        self.ids.remove(id);
    }

    pub fn contains(&self, id: &Uuid) -> bool {
        self.ids.contains(id)
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Replace contents with the supplied snapshot — used after every
    /// governance merge so the cache reads from the canonical state.
    pub fn replace<I: IntoIterator<Item = Uuid>>(&mut self, ids: I) {
        self.ids.clear();
        self.ids.extend(ids);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_remove_contains() {
        let mut set = PinnedSet::new();
        let id = Uuid::new_v4();
        assert!(!set.contains(&id));
        set.insert(id);
        assert!(set.contains(&id));
        assert_eq!(set.len(), 1);
        set.remove(&id);
        assert!(!set.contains(&id));
        assert!(set.is_empty());
    }

    #[test]
    fn replace_overwrites() {
        let mut set = PinnedSet::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        set.insert(a);
        set.replace([b]);
        assert!(!set.contains(&a));
        assert!(set.contains(&b));
    }
}
