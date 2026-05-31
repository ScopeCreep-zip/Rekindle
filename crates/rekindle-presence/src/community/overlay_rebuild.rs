//! Pure gossip-overlay rebuild planner.
//!
//! Computes the new overlay state + drained-pending envelopes from
//! the prior snapshot + freshly-scanned peers. The adapter exposes
//! `read_gossip_snapshot` (read the prior overlay) +
//! `apply_gossip_rebuild_plan` (atomic write of the new overlay).
//! Pre-port both responsibilities + the decision logic lived in
//! `presence_adapter/gossip_overlay.rs::rebuild_gossip_overlay`.
//!
//! Architecture A1/P4.1 — `pending_mesh_broadcasts` accumulated
//! while peers was empty must be re-sent once peers come online,
//! but NOT while the write-lock is held (would deadlock the
//! gossip relay's read lock). The plan returns the drained list
//! so the orchestrator can call `send_to_mesh_raw` after the
//! adapter releases the write lock.

use std::collections::{HashMap, VecDeque};

use rekindle_protocol::dht::community::envelope::SignedEnvelope;

use crate::deps::OnlineMemberSnapshot;

/// Snapshot of the prior gossip overlay state — `lamport_counter`
/// is preserved across the rebuild + `pending_mesh_broadcasts` is
/// drained when peers become available. `needs_initial_sync`
/// drives the initial-sync handshake trigger.
#[derive(Debug, Clone, Default)]
pub struct GossipOverlaySnapshot {
    pub lamport_counter: u64,
    pub needs_initial_sync: bool,
    pub pending_mesh_broadcasts: VecDeque<SignedEnvelope>,
}

/// Atomic-write payload for the rebuilt overlay. The adapter
/// applies this under one `community.communities` write lock so
/// the peers / online_members / lamport counter / needs_initial_sync
/// flags update together.
#[derive(Debug)]
pub struct GossipOverlayPlan {
    pub peers: HashMap<String, OnlineMemberSnapshot>,
    pub online_members: HashMap<String, OnlineMemberSnapshot>,
    pub lamport_counter: u64,
    pub needs_initial_sync: bool,
    pub remaining_pending: VecDeque<SignedEnvelope>,
}

/// Outcome consumed by the orchestrator: the write-plan +
/// drained-pending envelopes + needs_sync gate.
#[derive(Debug, Default)]
pub struct GossipRebuildOutcome {
    pub plan: Option<GossipOverlayPlan>,
    pub needs_sync: bool,
    pub drained_pending: Vec<SignedEnvelope>,
}

/// Compute the rebuild outcome for one tick.
///
/// `prior` carries the lamport counter + needs_initial_sync flag +
/// pending-mesh queue from the prior overlay. `selected` is the
/// fan-out subset (size = `gossip_degree`); `online_members` is the
/// full freshly-online set.
///
/// `needs_sync` fires when:
/// - prior overlay was `needs_initial_sync` (we haven't completed
///   the first round since coming online), AND
/// - at least one peer is now online (we have someone to ask).
///
/// `drained_pending` is non-empty when the rebuild moves us from
/// "no peers" to "some peers" — those queued envelopes get re-sent
/// AFTER the adapter releases the write lock.
#[must_use]
pub fn compute_rebuild_plan<S1, S2>(
    prior: GossipOverlaySnapshot,
    selected: HashMap<String, OnlineMemberSnapshot, S1>,
    online_members: HashMap<String, OnlineMemberSnapshot, S2>,
) -> GossipRebuildOutcome
where
    S1: std::hash::BuildHasher,
    S2: std::hash::BuildHasher,
{
    let online_count = online_members.len();
    let will_have_peers = !selected.is_empty();
    let lamport_counter = prior.lamport_counter;
    let needs_initial_sync = prior.needs_initial_sync;
    let GossipOverlaySnapshot {
        pending_mesh_broadcasts,
        ..
    } = prior;

    let (drained, remaining) = if will_have_peers {
        let drained: Vec<SignedEnvelope> = pending_mesh_broadcasts.into_iter().collect();
        (drained, VecDeque::new())
    } else {
        // Keep the queue intact for the next tick to retry.
        (Vec::new(), pending_mesh_broadcasts)
    };

    // Collect into the default-hasher map shape `GossipOverlayPlan`
    // stores. The generic `S1`/`S2` accept any hasher at the call
    // site (orchestrator hands in standard `HashMap`s today, but
    // tests can use ahash etc).
    let peers: HashMap<String, OnlineMemberSnapshot> = selected.into_iter().collect();
    let online_members: HashMap<String, OnlineMemberSnapshot> = online_members.into_iter().collect();
    let plan = GossipOverlayPlan {
        peers,
        online_members,
        lamport_counter,
        needs_initial_sync,
        remaining_pending: remaining,
    };

    GossipRebuildOutcome {
        plan: Some(plan),
        needs_sync: needs_initial_sync && online_count > 0,
        drained_pending: drained,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(blob: &[u8]) -> OnlineMemberSnapshot {
        OnlineMemberSnapshot {
            route_blob: blob.to_vec(),
            status: "online".to_string(),
            last_seen: 0,
        }
    }

    fn pending(payload: &[u8]) -> SignedEnvelope {
        SignedEnvelope {
            community_id: "c1".to_string(),
            sender_pseudonym: "me".to_string(),
            envelope_bytes: payload.to_vec(),
            signature: vec![0u8; 64],
            ttl: 5,
        }
    }

    #[test]
    fn empty_peers_preserves_pending_and_skips_sync() {
        let mut queue = VecDeque::new();
        queue.push_back(pending(b"queued"));
        let prior = GossipOverlaySnapshot {
            lamport_counter: 7,
            needs_initial_sync: true,
            pending_mesh_broadcasts: queue,
        };
        let outcome = compute_rebuild_plan(prior, HashMap::new(), HashMap::new());
        assert!(!outcome.needs_sync);
        assert!(outcome.drained_pending.is_empty());
        let plan = outcome.plan.expect("plan");
        assert_eq!(plan.remaining_pending.len(), 1);
        assert_eq!(plan.lamport_counter, 7);
        assert!(plan.peers.is_empty());
    }

    #[test]
    fn first_peer_triggers_drain_and_sync_when_needs_initial_sync() {
        let mut queue = VecDeque::new();
        queue.push_back(pending(b"a"));
        queue.push_back(pending(b"b"));
        let prior = GossipOverlaySnapshot {
            lamport_counter: 1,
            needs_initial_sync: true,
            pending_mesh_broadcasts: queue,
        };
        let mut selected = HashMap::new();
        selected.insert("peer1".to_string(), snapshot(&[1]));
        let mut online = HashMap::new();
        online.insert("peer1".to_string(), snapshot(&[1]));
        let outcome = compute_rebuild_plan(prior, selected, online);
        assert!(outcome.needs_sync);
        assert_eq!(outcome.drained_pending.len(), 2);
        let plan = outcome.plan.expect("plan");
        assert_eq!(plan.peers.len(), 1);
        assert_eq!(plan.online_members.len(), 1);
        assert!(plan.remaining_pending.is_empty());
    }

    #[test]
    fn peers_already_present_does_not_re_sync() {
        let prior = GossipOverlaySnapshot {
            lamport_counter: 42,
            needs_initial_sync: false,
            pending_mesh_broadcasts: VecDeque::new(),
        };
        let mut selected = HashMap::new();
        selected.insert("peer1".to_string(), snapshot(&[1]));
        let outcome = compute_rebuild_plan(prior, selected, HashMap::new());
        assert!(!outcome.needs_sync);
        assert!(outcome.drained_pending.is_empty());
        assert_eq!(outcome.plan.as_ref().unwrap().lamport_counter, 42);
    }

    #[test]
    fn lamport_counter_is_preserved_across_rebuild() {
        let prior = GossipOverlaySnapshot {
            lamport_counter: 12345,
            needs_initial_sync: false,
            pending_mesh_broadcasts: VecDeque::new(),
        };
        let outcome = compute_rebuild_plan(prior, HashMap::new(), HashMap::new());
        assert_eq!(outcome.plan.unwrap().lamport_counter, 12345);
    }
}
