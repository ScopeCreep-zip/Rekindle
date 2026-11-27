//! Dead member routes revived by a probe (plan RH1r-c S2/S3 on the
//! governance and bootstrap paths; inventory item 25; E3.4 S4/S5).
//!
//! A member whose route is alive never republishes it: its loopback passes,
//! so Veilid tells it nothing (0.5.8 `ping_validator.rs:800-830`), while one
//! importer's single remote test ping lost on a slow round trip removes the
//! route there (`ping_validator.rs:741-744`). The transport then refuses
//! the blob for its failure cache (`rekindle_route::dead_routes`). When a
//! read of the member's `M`/6 finds the same blob at the same seq one
//! re-fetch gap after the death, it is probed once by a call
//! ([`rekindle_transport::TransportNode::probe_route`]): Veilid allows the
//! re-import and tests it again (`route_remote.rs:21-40`), and an answer
//! makes it usable at once. A route that clears, revived here or
//! republished at a higher seq, is cached again and the mesh's held
//! broadcasts go then (E3.4 S5), not only on the next beat move.

use rekindle_route::dead_routes::DeadRouteVerdict;
use rekindle_transport::liveness::dead::Probed;
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

impl HostGovernance {
    /// Take `author`'s route `blob`, read at DHT `seq` with verdict `seen`,
    /// into `community`'s cache unless it is still `dead`; one that cleared
    /// sends the broadcasts the mesh held for want of peers.
    pub(crate) fn take_route(
        &self,
        community: &str,
        author: &PseudonymKey,
        seq: u64,
        blob: Vec<u8>,
        seen: DeadRouteVerdict,
        dead: bool,
    ) {
        if dead {
            return;
        }
        {
            let mut map = self.ctx.communities.map().write();
            if let Some(c) = map.get_mut(community) {
                c.member_routes.observe(author, seq, blob);
            }
        }
        if seen == DeadRouteVerdict::Cleared {
            self.drain_pending_mesh(community);
        }
    }

    /// Probe `author`'s dead route `blob` (read at `seq`) once; an answer
    /// takes it into the cache as cleared. `true` when it answered.
    pub(super) async fn probe_member_route(
        &self,
        community: &str,
        author: &PseudonymKey,
        seq: u64,
        blob: Vec<u8>,
    ) -> bool {
        let Ok(node) = self.transport() else {
            return false;
        };
        match node.probe_route(&blob).await {
            Probed::Answered => {
                tracing::info!(
                    community,
                    "a dead member route answered its probe: usable again"
                );
                self.take_route(
                    community,
                    author,
                    seq,
                    blob,
                    DeadRouteVerdict::Cleared,
                    false,
                );
                true
            }
            Probed::ImportFailed(reason) | Probed::Silent(reason) => {
                tracing::info!(community, %reason, "a dead member route did not answer its probe");
                false
            }
        }
    }

    /// [`Self::probe_member_route`] on the community's scope.
    pub(super) fn spawn_member_route_probe(
        &self,
        community: &str,
        author: &PseudonymKey,
        seq: u64,
        blob: Vec<u8>,
    ) {
        let governance = self.clone();
        let owned = community.to_owned();
        let author = author.clone();
        self.ctx
            .community_scope(community)
            .spawn_or_drop("dead member route probe", async move {
                governance
                    .probe_member_route(&owned, &author, seq, blob)
                    .await;
            });
    }
}
