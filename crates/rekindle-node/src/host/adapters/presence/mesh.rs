//! The community gossip mesh in the Host (plan E3.4 S5), one on both modes
//! (ADR 0010): PATH 2 of the three-path model, the D-peer epidemic
//! broadcast (TTL 5, dedup by the signed `message_id`).
//!
//! The mesh is the governance roster ∩ the members who may be present (a
//! beat showing them present, or none of theirs reached us yet: unknown is
//! not offline; plan E3.4 S3's beat view), each reached by
//! the route its `M`/6 published (the member route cache, plan E3.4 S4).
//! It is built from what the ring and the hints already delivered, so
//! nothing is polled for it (D4): the registry scan's overlay it replaces
//! was rebuilt by the presence poll and reached only members who wrote a
//! registry row.
//!
//! A sender keeps `gossip_degree(n)` of its `n` peers, ordered by
//! `blake3(scope ‖ peer)` (deterministic, no RNG on the send path). The
//! scope is the community **and the sender**: with the community alone
//! every member would rank the peers identically and choose the same D, so
//! a member past the first D + 1 would hear nothing; per sender, the D-out
//! graph differs at every node, as the epidemic broadcast needs.
//!
//! A member whose beat turns live while the cache holds no route of theirs
//! is read once by the S4 re-read rule ([`HostGovernance::read_route_cold`]),
//! on that beat's arrival, never on a timer. The broadcasts held while the
//! mesh was empty (`rekindle_gossip::MAX_PENDING_MESH`) go once it has a
//! peer.

use rekindle_gossip::PeerInfo;
use rekindle_types::id::PseudonymKey;

use super::super::governance::HostGovernance;

/// The rank of `peer` (pseudonym hex) in a mesh of `scope`.
fn rank(scope: &[u8], peer: &str) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(scope);
    hasher.update(peer.as_bytes());
    *hasher.finalize().as_bytes()
}

/// The `gossip_degree(n)` lowest-ranked of `peers` in a mesh of `scope`.
#[must_use]
pub(crate) fn pick(scope: &[u8], mut peers: Vec<PeerInfo>) -> Vec<PeerInfo> {
    peers.sort_by_cached_key(|p| rank(scope, &p.pseudonym_key));
    peers.truncate(rekindle_presence::community::gossip_degree(peers.len()));
    peers
}

impl HostGovernance {
    /// Our mesh in `community`: the roster's members whose beat is live and
    /// whose route the `M`/6 cache holds, picked for us. Empty when we hold
    /// no pseudonym there.
    #[must_use]
    pub fn mesh_peers(&self, community: &str) -> Vec<PeerInfo> {
        let Some(me) = self.me_in(community) else {
            return Vec::new();
        };
        let scope = [community.as_bytes(), me.0.as_slice()].concat();
        let peers = self
            .reachable_members(community)
            .into_iter()
            .map(|(pseudonym_key, route_blob)| PeerInfo {
                pseudonym_key,
                route_blob,
            })
            .collect();
        pick(&scope, peers)
    }

    /// `member`'s beat moved (by the ring or a hint): once it is live, a
    /// route the cache lacks is read from their `M`/6 once, and the held
    /// broadcasts go when the mesh now has a peer.
    pub(crate) async fn mesh_member_moved(&self, community: &str, member: &PseudonymKey) {
        let hex = member.to_hex();
        if !self.member_beat_shown(community, &hex).is_present() {
            return;
        }
        if self.cached_member_route(community, &hex).is_none()
            && self.read_route_cold(community, member).await.is_none()
        {
            return;
        }
        self.drain_pending_mesh(community);
    }

    /// Send a signed envelope (held for want of peers) to the mesh
    /// unchanged: its signature and id stay its author's.
    pub fn forward_signed(&self, signed: rekindle_codec::community::envelope::SignedEnvelope) {
        crate::daemon::gossip::forward(&self.ctx.gossip_tx, signed);
    }

    /// Send the broadcasts held for want of peers, unchanged (each keeps
    /// its signature and id), once the mesh has a peer.
    pub(crate) fn drain_pending_mesh(&self, community: &str) {
        if self.mesh_peers(community).is_empty() {
            return;
        }
        let held: Vec<_> = {
            let mut map = self.ctx.communities.map().write();
            map.get_mut(community)
                .and_then(|c| c.gossip.as_mut())
                .map(|g| std::mem::take(&mut g.pending_mesh_broadcasts))
                .unwrap_or_default()
                .into()
        };
        if held.is_empty() {
            return;
        }
        tracing::info!(
            community,
            held = held.len(),
            "mesh: sending the held broadcasts"
        );
        for signed in held {
            self.forward_signed(signed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peers(n: usize) -> Vec<PeerInfo> {
        (0..n)
            .map(|i| PeerInfo {
                pseudonym_key: format!("{i:064x}"),
                route_blob: vec![1],
            })
            .collect()
    }

    /// The pick keeps `gossip_degree(n)` and is the same for the same scope.
    #[test]
    fn the_pick_is_deterministic_and_degree_bound() {
        let a = pick(b"c\x01", peers(20));
        let b = pick(b"c\x01", peers(20).into_iter().rev().collect());
        assert_eq!(a.len(), 6);
        assert_eq!(
            a.iter().map(|p| &p.pseudonym_key).collect::<Vec<_>>(),
            b.iter().map(|p| &p.pseudonym_key).collect::<Vec<_>>()
        );
        assert_eq!(pick(b"c", peers(3)).len(), 3);
        assert_eq!(pick(b"c", peers(80)).len(), 8);
    }

    /// Two senders in one community do not pick the same peers: the scope
    /// holds the sender, so every member of a large community is reached.
    #[test]
    fn senders_pick_different_peers() {
        let union: std::collections::HashSet<String> = (0u8..40)
            .flat_map(|sender| pick(&[b"c".as_slice(), &[sender]].concat(), peers(40)))
            .map(|p| p.pseudonym_key)
            .collect();
        assert!(union.len() > 30, "{}", union.len());
    }
}
