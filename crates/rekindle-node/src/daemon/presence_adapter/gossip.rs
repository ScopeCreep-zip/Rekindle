//! Gossip overlay reads and mesh broadcast.
//!
//! The overlay is the daemon's live view of who is reachable right now,
//! maintained by `SubscriptionManager`'s meshes. It is deliberately
//! distinct from the roster in `community_runtime`: the roster answers
//! "who is a member" (registry rows, changes on join/leave/ban), the
//! overlay answers "who can I reach" (gossip heartbeats, changes by the
//! second). Conflating them is how an offline member gets treated as
//! departed.

use std::collections::HashMap;

use rekindle_presence::community::{GossipOverlayPlan, GossipOverlaySnapshot};
use rekindle_presence::deps::OnlineMember;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

use super::DaemonPresenceAdapter;

/// Convert the crate's snapshots into the mesh's own member type.
fn to_mesh_members(
    from: HashMap<String, OnlineMember>,
) -> HashMap<String, rekindle_transport::OnlineMember> {
    from.into_iter()
        .map(|(pseudonym, snapshot)| {
            (
                pseudonym,
                rekindle_transport::OnlineMember {
                    route_blob: snapshot.route_blob,
                    status: snapshot.status,
                    last_seen: snapshot.last_seen,
                    // Carried through rather than dropped: the snapshot
                    // decoded both off the registry row, and a daemon
                    // client needs them to show where a member is
                    // focused and when they were last active.
                    location: snapshot.location,
                    last_active: snapshot.last_active,
                },
            )
        })
        .collect()
}

impl DaemonPresenceAdapter {
    /// Fan a presence-orchestrator envelope out over the community mesh.
    ///
    /// Queued for the gossip worker rather than translated here. This
    /// used to match `PresenceUpdate` and `Control(SyncRequest)` by name
    /// and error on anything else — one of three such partial
    /// translations on this track, each into transport's postcard
    /// `GossipPayload`, which no desktop peer could read. See
    /// `daemon::gossip`.
    pub(super) fn send_to_mesh_impl(&self, community_id: &str, envelope: &CommunityEnvelope) {
        crate::daemon::gossip::send(&self.ctx.gossip_tx, community_id, envelope);
    }

    /// The overlay's rebuild inputs: gossip clock, sync gate, and any
    /// envelopes queued while the mesh had no peers.
    pub(super) fn gossip_snapshot_impl(&self, community_id: &str) -> GossipOverlaySnapshot {
        let guard = self.ctx.subscriptions.read();
        let Some(manager) = guard.as_ref() else {
            return GossipOverlaySnapshot::default();
        };
        let meshes = manager.meshes().read();
        let Some(mesh) = meshes.get(community_id) else {
            return GossipOverlaySnapshot::default();
        };
        GossipOverlaySnapshot {
            lamport_counter: mesh.clock.current(),
            // The daemon syncs on unlock via `SubscriptionManager`
            // rather than gating on this flag.
            needs_initial_sync: false,
            // No queue on this track: `BroadcastManager` sends through
            // the mesh directly, so there is nothing held back to
            // replay once peers appear.
            pending_mesh_broadcasts: std::collections::VecDeque::new(),
        }
    }

    /// The live online set, read straight from the mesh.
    fn online_from_mesh(&self, community_id: &str) -> HashMap<String, OnlineMember> {
        let guard = self.ctx.subscriptions.read();
        let Some(manager) = guard.as_ref() else {
            return HashMap::new();
        };
        let meshes = manager.meshes().read();
        let Some(mesh) = meshes.get(community_id) else {
            return HashMap::new();
        };
        mesh.online_members
            .iter()
            .map(|(pseudonym, member)| {
                (
                    pseudonym.clone(),
                    OnlineMember {
                        route_blob: member.route_blob.clone(),
                        status: member.status.clone(),
                        last_seen: member.last_seen,
                        // Location rides in the MEK-encrypted
                        // SessionExtras, which the scan decrypts; the
                        // gossip overlay carries none.
                        location: None,
                        last_active: member.last_seen,
                    },
                )
            })
            .collect()
    }

    /// Fold peers the gossip overlay has heard from into the scan's
    /// online set.
    ///
    /// Gossip is the *fast* path of three-path delivery and the registry
    /// scan is the *consistent* one, so a peer who joined since the last
    /// tick is reachable by gossip before their registry row is visible.
    /// Dropping them until the next scan would make a fresh joiner
    /// unreachable for a full poll interval.
    pub(super) fn extend_online_impl(
        &self,
        community_id: &str,
        online_members: &mut HashMap<String, OnlineMember>,
        my_pseudonym: &str,
        eviction_threshold_secs: u64,
    ) {
        let now = rekindle_utils::timestamp_secs();
        for (pseudonym, member) in self.online_from_mesh(community_id) {
            if pseudonym == my_pseudonym {
                continue;
            }
            if now.saturating_sub(member.last_seen) > eviction_threshold_secs {
                continue;
            }
            online_members.entry(pseudonym).or_insert(member);
        }
    }

    /// Members the overlay considers gone that the scan still lists.
    pub(super) fn gossip_offline_diff_impl(
        &self,
        community_id: &str,
        online_members: &HashMap<String, OnlineMember>,
        my_pseudonym: &str,
    ) -> Vec<String> {
        self.online_from_mesh(community_id)
            .into_keys()
            .filter(|pseudonym| {
                pseudonym != my_pseudonym && !online_members.contains_key(pseudonym)
            })
            .collect()
    }

    /// Apply the crate's computed overlay rebuild atomically.
    ///
    /// The plan is a whole-set replacement, and applying it under one
    /// write lock is the point: the online set, peer set and gossip
    /// clock have to move together or a concurrent broadcast can pick a
    /// peer list from before the rebuild and a Lamport stamp from after.
    pub(super) fn apply_gossip_plan_impl(&self, community_id: &str, plan: GossipOverlayPlan) {
        let guard = self.ctx.subscriptions.read();
        let Some(manager) = guard.as_ref() else {
            return;
        };
        let mut meshes = manager.meshes().write();
        let Some(mesh) = meshes.get_mut(community_id) else {
            return;
        };
        mesh.online_members = to_mesh_members(plan.online_members);
        mesh.peers = to_mesh_members(plan.peers);
        // `merge`, not an assignment: it is the drift-capped advance
        // (4.8), so a plan carrying a wild counter cannot push this
        // mesh's clock somewhere it can never come back from. A plan
        // that is merely behind is absorbed without moving us backwards.
        mesh.clock.merge(plan.lamport_counter);
    }
}
