//! Holding an outbox entry until its key arrives (plan E3.3, D23 "messages
//! that arrived before their key"; ADR 0018). An entry whose tag maps to
//! no channel we read either belongs to a channel we do not read (or to a
//! time before our keys), and the cursor moves past it, or was sealed
//! under a community generation we are owed and do not hold yet, and the
//! cursor stays on it until the key's install reruns the catch-up
//! (`governance/mek.rs`).
//!
//! We are owed a generation when the merged state has converged on it
//! (`GovernanceState::mek_generation`; ADR 0018 rule 7: every bump is a
//! pure function of the op set, used only once confirmed) and it is not
//! older than the first community key we hold (a joiner gets the current
//! generation's wraps only, ADR 0018 "Admission never bumps"). That bound
//! is what keeps an author's outbox from wedging: an entry of a private
//! channel we were removed from is sealed under that channel's scope,
//! whose generations no converged community bump owes us; if its number
//! collides with an owed community generation, the hold ends when that
//! key arrives, the tag still maps nowhere, and the cursor moves on.
//! Channel-scope generations enter the merged state with E3.5's ballots
//! (ADR 0018 rule 1 covers both scopes); until then only a community
//! generation holds an entry.

use rekindle_types::channel_keys::{KeyEpoch, KeyScope};

use rekindle_channel::ChannelMessagingDeps as _;

use super::HostChannel;

/// Whether an entry at `generation` waits for its key: no community key
/// is held yet (`first_held` none), or `generation` is one the merged
/// state converged on (`converged`), not older than our first key, and not
/// held.
#[must_use]
pub(crate) fn waits_for_key(
    generation: u64,
    first_held: Option<u64>,
    converged: u64,
    held: impl Fn(u64) -> bool,
) -> bool {
    let Some(first) = first_held else {
        return true;
    };
    (first..=converged).contains(&generation) && !held(generation)
}

impl HostChannel {
    /// Whether an entry of `community_id` sealed at `generation`, whose
    /// tag maps to no channel we read, waits for its key ([`waits_for_key`]).
    pub(super) fn entry_waits_for_key(&self, community_id: &str, generation: u64) -> bool {
        let keys = self.keys();
        let held = |g: u64| {
            keys.key(community_id, KeyScope::Community, KeyEpoch(g))
                .is_some()
        };
        let first_held = (keys.current_epoch(community_id, KeyScope::Community))
            .and_then(|current| (1..=current.0).find(|g| held(*g)));
        let converged = (self.governance_state(community_id)).map_or(0, |s| s.mek_generation);
        waits_for_key(generation, first_held, converged, held)
    }
}

#[cfg(test)]
mod tests {
    use super::waits_for_key;

    /// Generations 2 and 3 held, the state converged on 4.
    fn held(g: u64) -> bool {
        (2..=3).contains(&g)
    }

    #[test]
    fn a_converged_generation_not_held_yet_waits() {
        assert!(waits_for_key(4, Some(2), 4, held));
    }

    #[test]
    fn before_any_key_every_entry_waits() {
        assert!(waits_for_key(1, None, 4, held));
    }

    /// A private channel's generation above every converged community
    /// bump (a channel we were removed from) never holds the cursor.
    #[test]
    fn a_generation_no_converged_bump_owes_moves_on() {
        assert!(!waits_for_key(7, Some(2), 4, held));
    }

    #[test]
    fn a_generation_before_our_first_key_moves_on() {
        assert!(!waits_for_key(1, Some(2), 4, held));
    }

    #[test]
    fn a_held_generation_whose_tag_maps_nowhere_moves_on() {
        assert!(!waits_for_key(3, Some(2), 4, held));
    }
}
