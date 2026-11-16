//! Our own removal from a community (plan E3.2 S9) as [`CommunityStore`]
//! records it, and whose traffic the community takes.

use rekindle_types::id::PseudonymKey;

use super::super::state::OwnRemoval;
use super::CommunityStore;

impl CommunityStore {
    /// Note our own removal (plan E3.2 S9 own removal): whether the
    /// community was not marked removed before.
    pub fn note_own_removal(&self, community_id: &str, removal: OwnRemoval) -> bool {
        self.write(community_id, |c| c.own_removal.replace(removal).is_none())
    }

    /// Clear our removal mark: the merged state no longer removes us (the
    /// removal was void, strong removal). Whether it was marked.
    pub fn clear_own_removal(&self, community_id: &str) -> bool {
        self.read(community_id, |c| c.own_removal.is_some()) == Some(true)
            && self.write(community_id, |c| c.own_removal.take().is_some())
    }

    /// The lock over whether the community's network side is torn down for
    /// our removal.
    #[must_use]
    pub fn removal_gate(
        &self,
        community_id: &str,
    ) -> Option<std::sync::Arc<tokio::sync::Mutex<bool>>> {
        self.read(community_id, |c| std::sync::Arc::clone(&c.removal_gate))
    }

    /// Our own removal from the community, if the merge found one.
    #[must_use]
    pub fn own_removal(&self, community_id: &str) -> Option<OwnRemoval> {
        self.read(community_id, |c| c.own_removal.clone()).flatten()
    }

    /// Whether we take `author`'s traffic for the community (gossip, its
    /// relay, key transfers): never once we were removed from it, nor from
    /// a member who departed (plan E3.2 S9, amended 2026-11-15). A relay
    /// keeps its author's signature, so the author is who is judged. A
    /// community the store does not hold is judged by its own handlers.
    #[must_use]
    pub fn takes_traffic_from(&self, community_id: &str, author: &PseudonymKey) -> bool {
        self.read(community_id, |c| {
            c.own_removal.is_none()
                && !(c.governance_state.as_ref()).is_some_and(|state| state.is_departed(author))
        })
        .unwrap_or(true)
    }
}
