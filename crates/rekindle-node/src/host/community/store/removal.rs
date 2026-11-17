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

    /// Keep `name` as the name of our remover `by`, when the mark still
    /// names `by` and holds no name (A/B R5-2): whether it was kept.
    pub fn name_own_removal(&self, community_id: &str, by: &PseudonymKey, name: String) -> bool {
        self.write(community_id, |c| match c.own_removal.as_mut() {
            Some(removal) if removal.by.as_ref() == Some(by) && removal.by_name.is_none() => {
                removal.by_name = Some(name);
                true
            }
            _ => false,
        })
    }

    /// The display name `by` goes by in the community as we know it now:
    /// its verified card, else its profile (A/B R5-2: the remover is not
    /// always a member whose card the admission queue read).
    #[must_use]
    pub fn removal_name_of(&self, community_id: &str, by: &PseudonymKey) -> Option<String> {
        self.member_card_name(community_id, by).or_else(|| {
            self.read(community_id, |c| {
                (c.member_profiles.get(&by.to_hex())).and_then(|p| p.display_name.clone())
            })
            .flatten()
            .filter(|name| !name.is_empty())
        })
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

    /// Whether we relay `author`'s gossip for the community one hop
    /// further: only for a community we hold, whose traffic we take
    /// ([`Self::takes_traffic_from`]). A community we left or forgot is
    /// gone from the store, and its remaining members' gossip that still
    /// reaches us (their peer lists hold us until they refresh) is not
    /// ours to amplify, as Signal's left group sends nothing (A/B F7-4).
    #[must_use]
    pub fn relays_for(&self, community_id: &str, author: &PseudonymKey) -> bool {
        self.read(community_id, |_| ()).is_some() && self.takes_traffic_from(community_id, author)
    }
}
