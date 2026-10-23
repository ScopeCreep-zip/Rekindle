//! The undo of an action that ran (plan E1.1, ADR 0019): the host computes
//! the inverse from live state when it selects the action
//! (`intent_select::inverse`), keeps it here for a short while, owned like
//! a held action, and answers the offer with the response. `Undo { id }`
//! takes it back out and sends it through the gate as a request of its
//! own, so the inverse is classified on its own.

use std::collections::VecDeque;

use parking_lot::Mutex;
use rekindle_idempotency::intent_gate::{mint_intent_id, GateOwner};
use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::intent::{IntentId, IntentKind, UndoOffer};

use super::gate::IntentOwner;

/// How long an offer lasts: Gmail's default undo-send window.
pub(crate) const UNDO_WINDOW_MS: u64 = 10_000;
/// The most offers held at once; the oldest goes first.
const CAPACITY: usize = 32;

struct Entry {
    id: IntentId,
    owner: IntentOwner,
    inverse: IpcRequest,
    expires_at_ms: u64,
}

/// The offers the host holds.
#[derive(Default)]
pub struct UndoCache {
    entries: Mutex<VecDeque<Entry>>,
}

impl UndoCache {
    /// Keep `inverse` for `owner`, and the offer that names it, whose id
    /// comes from the host's clock (`now_ms`) and RNG port (plan C9.2).
    pub(crate) fn offer(
        &self,
        owner: IntentOwner,
        inverse: IpcRequest,
        kind: IntentKind,
        now_ms: u64,
        rng: &dyn rekindle_utils::RandomSource,
    ) -> UndoOffer {
        let id = mint_intent_id(now_ms, rng);
        let expires_at_ms = now_ms + UNDO_WINDOW_MS;
        let mut entries = self.entries.lock();
        entries.retain(|entry| entry.expires_at_ms > now_ms);
        if entries.len() >= CAPACITY {
            entries.pop_front();
        }
        entries.push_back(Entry {
            id,
            owner,
            inverse,
            expires_at_ms,
        });
        UndoOffer {
            id,
            inverse: kind,
            expires_at_ms,
        }
    }

    /// The inverse offered as `id`, once, to whoever may act for its
    /// owner, while it lasts.
    pub(crate) fn take(
        &self,
        id: IntentId,
        caller: &IntentOwner,
        now_ms: u64,
    ) -> Option<IpcRequest> {
        let mut entries = self.entries.lock();
        entries.retain(|entry| entry.expires_at_ms > now_ms);
        let at = entries
            .iter()
            .position(|entry| entry.id == id && caller.may_act_for(&entry.owner))?;
        entries.remove(at).map(|entry| entry.inverse)
    }

    /// Drop every offer whose owner matches (lock, logout, a closed
    /// connection).
    pub(crate) fn clear_matching(&self, matches: impl Fn(&IntentOwner) -> bool) {
        self.entries.lock().retain(|entry| !matches(&entry.owner));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_ipc::ConnId;
    use rekindle_types::presence::SessionStatus;

    fn owner(conn: u64) -> IntentOwner {
        IntentOwner {
            conn: Some(ConnId::Bus(conn)),
            identity: "me".into(),
        }
    }

    fn back() -> IpcRequest {
        IpcRequest::PresenceSet {
            status: SessionStatus::Online,
        }
    }

    #[test]
    fn an_offer_is_taken_once_by_its_owner_while_it_lasts() {
        let cache = UndoCache::default();
        let offer = cache.offer(owner(1), back(), IntentKind::SetStatus, 1_000, &rekindle_utils::OsRandom);
        assert_eq!(offer.expires_at_ms, 1_000 + UNDO_WINDOW_MS);
        assert!(
            cache.take(offer.id, &owner(2), 1_001).is_none(),
            "another connection"
        );
        assert!(cache.take(offer.id, &owner(1), 1_001).is_some());
        assert!(
            cache.take(offer.id, &owner(1), 1_002).is_none(),
            "taken once"
        );

        let late = cache.offer(owner(1), back(), IntentKind::SetStatus, 1_000, &rekindle_utils::OsRandom);
        assert!(cache
            .take(late.id, &owner(1), 1_000 + UNDO_WINDOW_MS)
            .is_none());
    }

    #[test]
    fn a_lock_drops_its_identitys_offers() {
        let cache = UndoCache::default();
        let offer = cache.offer(owner(1), back(), IntentKind::SetStatus, 0, &rekindle_utils::OsRandom);
        cache.clear_matching(|o| o.identity == "me");
        assert!(cache.take(offer.id, &owner(1), 1).is_none());
    }
}
