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
use std::sync::Arc;

use rekindle_presence::community::{GossipOverlayPlan, GossipOverlaySnapshot};
use rekindle_presence::deps::OnlineMemberSnapshot;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use super::DaemonPresenceAdapter;

/// Convert the crate's snapshots into the mesh's own member type.
fn to_mesh_members(
    from: HashMap<String, OnlineMemberSnapshot>,
) -> HashMap<String, rekindle_transport::OnlineMember> {
    from.into_iter()
        .map(|(pseudonym, snapshot)| {
            (
                pseudonym,
                rekindle_transport::OnlineMember {
                    route_blob: snapshot.route_blob,
                    status: snapshot.status,
                    last_seen: snapshot.last_seen,
                },
            )
        })
        .collect()
}

impl DaemonPresenceAdapter {
    /// Fan a presence-orchestrator envelope out over the community mesh.
    ///
    /// Two kinds arrive here and no others: `PresenceUpdate` (a
    /// top-level envelope variant, not a control payload) and
    /// `Control(SyncRequest)`. Matching them by name rather than
    /// accepting anything keeps an unhandled kind loud instead of
    /// silently unsent.
    ///
    /// Handles are cloned out and the send is spawned: the trait method
    /// is sync and this runs on the tokio runtime, so blocking here
    /// would stall a worker thread.
    pub(super) fn send_to_mesh_impl(&self, community_id: &str, envelope: &CommunityEnvelope) {
        let Some(node) = self.transport() else { return };
        let Some(signing_key) = self.ctx.signing_key.read().as_ref().map(|k| *k.as_bytes()) else {
            return;
        };
        let (meshes, rate_limiter) = {
            let guard = self.ctx.broadcast_mgr.read();
            let Some(manager) = guard.as_ref() else {
                return;
            };
            (
                Arc::clone(manager.meshes()),
                Arc::clone(manager.rate_limiter()),
            )
        };
        let community_id = community_id.to_string();

        match envelope {
            CommunityEnvelope::PresenceUpdate {
                pseudonym_key,
                status,
                route_blob,
                ..
            } => {
                let sender = pseudonym_key.clone();
                let status = status.clone();
                let route_blob = route_blob.clone();
                tokio::spawn(async move {
                    // Returns `None` when rate-limited, which is a
                    // normal outcome at heartbeat cadence, not a failure.
                    let report = rekindle_transport::broadcast::gossip::presence_update(
                        &node,
                        &meshes,
                        &rate_limiter,
                        &community_id,
                        &sender,
                        &status,
                        None,
                        None,
                        None,
                        None,
                        route_blob,
                        &signing_key,
                    )
                    .await;
                    if let Some(report) = report {
                        if report.delivered == 0 && !report.failures.is_empty() {
                            tracing::debug!(
                                community = %community_id,
                                failures = report.failures.len(),
                                "gossip: presence update reached no peers"
                            );
                        }
                    }
                });
            }
            CommunityEnvelope::Control(ControlPayload::SyncRequest {
                channel_id,
                since_timestamp,
            }) => {
                let sender = self.my_pseudonym_impl(&community_id);
                if sender.is_empty() {
                    return;
                }
                let channel_id = channel_id.clone();
                let since = *since_timestamp;
                tokio::spawn(async move {
                    let report = rekindle_transport::broadcast::gossip::sync_request(
                        &node,
                        &meshes,
                        &community_id,
                        &sender,
                        &channel_id,
                        since,
                        &signing_key,
                    )
                    .await;
                    if report.delivered == 0 && !report.failures.is_empty() {
                        tracing::debug!(
                            community = %community_id,
                            channel = %channel_id,
                            "gossip: sync request reached no peers"
                        );
                    }
                });
            }
            _ => tracing::debug!(
                community = %&community_id[..16.min(community_id.len())],
                "send_to_mesh: unexpected envelope on the presence path"
            ),
        }
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
    fn online_from_mesh(&self, community_id: &str) -> HashMap<String, OnlineMemberSnapshot> {
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
                    OnlineMemberSnapshot {
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
        online_members: &mut HashMap<String, OnlineMemberSnapshot>,
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
        online_members: &HashMap<String, OnlineMemberSnapshot>,
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
