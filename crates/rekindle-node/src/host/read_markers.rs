//! Read markers advanced in memory and not yet written (plan E3.1, D25):
//! the ≤ 30 s write-behind. A pending marker only ever rises; the
//! write-behind (`adapters::governance::unread::write_behind`) moves them to
//! `rekindle_db::repo::read_markers`, which keeps the larger of the two.

use std::collections::BTreeMap;

use rekindle_db::repo::read_markers::Scope;

/// Markers waiting for the write-behind, in scope order.
#[derive(Debug, Default)]
pub struct PendingMarkers {
    inner: parking_lot::Mutex<BTreeMap<Scope, u64>>,
}

impl PendingMarkers {
    /// Raise the pending marker on `scope` to `seq`; a lower one changes
    /// nothing.
    pub fn note(&self, scope: Scope, seq: u64) {
        let mut inner = self.inner.lock();
        let held = inner.entry(scope).or_insert(seq);
        *held = (*held).max(seq);
    }

    /// The pending marker on `scope`, if one waits.
    #[must_use]
    pub fn get(&self, scope: &Scope) -> Option<u64> {
        self.inner.lock().get(scope).copied()
    }

    /// Every pending marker, leaving none.
    #[must_use]
    pub fn take(&self) -> BTreeMap<Scope, u64> {
        std::mem::take(&mut *self.inner.lock())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_marker_only_rises() {
        let pending = PendingMarkers::default();
        let scope = Scope::Dm {
            peer_key: "p".into(),
        };
        pending.note(scope.clone(), 4);
        pending.note(scope.clone(), 2);
        assert_eq!(pending.get(&scope), Some(4));
        assert_eq!(
            pending.take().into_iter().collect::<Vec<_>>(),
            vec![(scope.clone(), 4)]
        );
        assert_eq!(pending.get(&scope), None);
    }
}
