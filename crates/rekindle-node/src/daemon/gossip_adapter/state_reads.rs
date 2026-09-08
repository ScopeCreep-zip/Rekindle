//! Overlay and identity reads.

use std::collections::{HashMap, HashSet};

use rekindle_gossip::PeerInfo;

use super::DaemonGossipAdapter;

impl DaemonGossipAdapter {
    pub(super) fn my_pseudonym_key_impl(&self, community_id: &str) -> String {
        self.ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(community_id))
            .map(|m| m.pseudonym_key.clone())
            .unwrap_or_default()
    }

    pub(super) fn identity_secret_impl(&self) -> Option<[u8; 32]> {
        self.ctx.signing_key.read().as_ref().map(|k| *k.as_bytes())
    }

    /// The fan-out targets for this community.
    ///
    /// `GossipMesh.peers` is the selected subset (size = fan-out
    /// degree); `online_members` is everyone reachable. The crate
    /// re-sorts and truncates to the degree itself, so handing it the
    /// full online set rather than the pre-selected subset lets its
    /// reliability weighting actually choose — with only `peers` it
    /// would be re-ranking a list something else already narrowed.
    pub(super) fn current_peers_impl(&self, community_id: &str) -> Option<Vec<PeerInfo>> {
        let guard = self.ctx.broadcast_mgr.read();
        let manager = guard.as_ref()?;
        let meshes = manager.meshes().read();
        let mesh = meshes.get(community_id)?;
        Some(
            mesh.online_members
                .iter()
                .filter(|(_, member)| !member.route_blob.is_empty())
                .map(|(pseudonym, member)| PeerInfo {
                    pseudonym_key: pseudonym.clone(),
                    route_blob: member.route_blob.clone(),
                })
                .collect(),
        )
    }

    /// Per-peer delivery reliability (§14.5, gossip topology
    /// optimization).
    ///
    /// The daemon keeps no delivery history — it has no SQLite, and
    /// `record_peer_reliability` has nowhere durable to write. Returning
    /// an empty map is honest: the crate's `sort_peers_by_reliability`
    /// treats an absent score as neutral, so fan-out selection stays
    /// uniform rather than being skewed by fabricated numbers. The
    /// "ziplines" the architecture describes emerge on the desktop
    /// track, which does have the store.
    pub(super) fn peer_reliability_scores_impl(_community_id: &str) -> HashMap<String, f64> {
        HashMap::new()
    }

    pub(super) fn online_member_status_impl(
        &self,
        community_id: &str,
        peer_key: &str,
    ) -> Option<String> {
        let guard = self.ctx.broadcast_mgr.read();
        let manager = guard.as_ref()?;
        let meshes = manager.meshes().read();
        meshes
            .get(community_id)?
            .online_members
            .get(peer_key)
            .map(|member| member.status.clone())
    }
}

impl DaemonGossipAdapter {
    /// A peer's current route blob, re-read from the registry.
    ///
    /// Reads the peer's **own** presence row rather than any aggregate:
    /// the row is signed by the pseudonym it names, so a route taken
    /// from it cannot have been planted by another slot-keypair holder.
    /// `parse_and_classify_row` performs that W26 check, which is why
    /// this goes through it rather than deserialising the JSON directly.
    ///
    /// `force_refresh` is on: the whole reason this is called is that
    /// the cached blob failed, so a local copy would return the same
    /// stale route.
    pub(super) async fn resolve_peer_route_impl(
        &self,
        community_id: &str,
        peer_pseudonym: &str,
    ) -> Option<Vec<u8>> {
        let node = self.transport()?;
        let (registry_key, subkey) = {
            let members = self.ctx.community_runtime.members(community_id);
            let record = members.get(peer_pseudonym)?;
            let registry_key = self.segment_registry_key(community_id, record.segment_index)?;
            (registry_key, record.subkey_index)
        };

        let raw = rekindle_transport::broadcast::dht_writes::get(
            node.as_ref(),
            &registry_key,
            subkey,
            true,
        )
        .await
        .ok()
        .flatten()?;

        let banned: HashSet<String> = self
            .ctx
            .community_runtime
            .governance_state(community_id)
            .map(|s| s.bans.iter().map(|p| hex::encode(p.0)).collect())
            .unwrap_or_default();

        match rekindle_presence::parse_and_classify_row(&raw, &banned, 0, 0) {
            rekindle_presence::ClassifiedRow::Accepted(row)
                if row.pseudonym_hex == peer_pseudonym && !row.presence.route_blob.is_empty() =>
            {
                Some(row.presence.route_blob)
            }
            // A row that names someone else is not this peer's route —
            // the slot may have been reclaimed since the roster was
            // built. Anything else failed verification or is banned.
            _ => None,
        }
    }

    /// The registry record key for one segment: segment 0 comes from
    /// our membership, later segments from merged governance.
    fn segment_registry_key(&self, community_id: &str, segment_index: u32) -> Option<String> {
        if segment_index == 0 {
            return self
                .ctx
                .session
                .read()
                .as_ref()
                .and_then(|s| s.community(community_id))
                .map(|m| m.registry_key.clone());
        }
        self.ctx
            .community_runtime
            .governance_state(community_id)?
            .segments
            .iter()
            .find(|s| s.segment_index == segment_index)
            .map(|s| s.registry_key.clone())
    }
}
