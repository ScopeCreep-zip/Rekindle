//! The bootstrap exchange (plan E3.2 join step 5, ADR 0011 "Join"): our
//! `MemberJoined` handed to the inviter by `BootstrapRequest @42`, which
//! verifies, stores and merges it, answers its frontier and pushes the
//! community key (D16). The join itself stands from the DHT; the exchange
//! is how a member first learns of it when no member was online to take
//! its gossip: every member's sweep and ring read only records the member
//! already knows (`rekindle_governance_runtime::anti_entropy`,
//! `watch_ring`), and no record a member reads names a joiner it has not
//! merged. So an exchange the inviter did not answer is a `bootstrap`
//! outbox row (D22), tried again until it answers
//! (`crate::host::adapters::outbox::bootstrap`); once one member merged
//! the op, its frontier cites it and every other member's sweep finds it.

use rekindle_governance_runtime::op_log::HeldOp;
use rekindle_governance_runtime::{BootstrapJoiner, GovernanceRuntimeError};
use rekindle_types::governance::OpRef;
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

/// How one bootstrap exchange ended.
pub(crate) enum Exchange {
    /// A member answered our `MemberJoined` with its frontier; the ops our
    /// walk from it found are stored and the community rebuilt.
    Merged,
    /// The inviter answered and refused it: every member's merge holds it
    /// invalid.
    Refused(GovernanceRuntimeError),
    /// Nobody answered (the inviter is away, holds no route, or was partial
    /// and no member answered): the join stands from the DHT and the
    /// exchange is tried again.
    Unanswered(String),
}

impl HostGovernance {
    /// Do we hold `community`, not removed from it (plan E3.2 S9: a removed
    /// member announces nothing)?
    pub(crate) fn holds_community(&self, community: &str) -> bool {
        self.ctx.communities.map().read().contains_key(community)
            && self.ctx.communities.own_removal(community).is_none()
    }

    /// The inviter answered a retried exchange and refused our
    /// `MemberJoined`: withdraw the join as the join itself does when it is
    /// refused at once (our `MemberLeft`, the community forgotten), never a
    /// membership no member admits.
    pub(crate) async fn withdraw_refused_join(
        &self,
        community: &str,
        refused: &GovernanceRuntimeError,
    ) {
        let membership = (self.ctx.communities.map().read())
            .get(community)
            .map(super::loader::membership);
        let Some(membership) = membership else {
            return;
        };
        tracing::warn!(community, error = %refused, "join refused by the inviter, withdrawn");
        if let Err(error) = self.leave(&membership).await {
            tracing::warn!(community, %error, "refused join not withdrawn");
        }
    }

    /// The inviter's pseudonym: the member whose record is `inviter_record`
    /// in the merged state.
    fn inviter_of(&self, community: &str, inviter_record: &str) -> Option<PseudonymKey> {
        let state = self.ctx.communities.governance_state(community)?;
        (state.members.iter())
            .find(|(_, m)| m.member_record == inviter_record)
            .map(|(p, _)| p.clone())
    }

    /// The inviter's current route from its `M`/6 (forced: it changes when
    /// the inviter comes back); `None` while it holds none or it does not
    /// verify.
    pub(crate) async fn inviter_route(
        &self,
        community: &str,
        inviter_record: &str,
    ) -> Option<Vec<u8>> {
        use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
        let inviter = self.inviter_of(community, inviter_record)?;
        let port = self.record_port().ok()?;
        let read = rekindle_governance_runtime::member_routes::read_member_route(
            &*port,
            community,
            inviter_record,
            &inviter,
        )
        .await;
        match read {
            Ok(route) => route.filter(|r| !r.is_empty()),
            Err(error) => {
                tracing::debug!(community, %error, "the inviter's route was not read");
                None
            }
        }
    }

    /// One bootstrap exchange: `joined` (our `MemberJoined`) to the inviter
    /// at `inviter_route` (its record `inviter_record`); a partial inviter
    /// hands it to the members our walk shows (S7c.8 (i),
    /// `rekindle_governance_runtime::bootstrap_peers`).
    pub(crate) async fn bootstrap_exchange(
        &self,
        community: &str,
        inviter_route: &[u8],
        inviter_record: &str,
        joined: &HeldOp,
    ) -> Exchange {
        use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
        if inviter_route.is_empty() {
            return Exchange::Unanswered("the inviter holds no route".into());
        }
        let joiner = BootstrapJoiner {
            pseudonym: hex::encode(joined.author.0),
            member_joined: OpRef {
                op_hash: joined.hash,
                author: joined.author.clone(),
                author_record: joined.author_record.clone(),
                author_seq: joined.seq,
            },
            member_joined_op: joined.encoded.clone(),
            route: self.our_route_blob_impl(),
        };
        let walked = match rekindle_governance_runtime::request_bootstrap(
            self,
            inviter_route,
            inviter_record,
            community,
            &joiner,
        )
        .await
        {
            Ok(walked) => walked,
            Err(e @ GovernanceRuntimeError::BootstrapRefused(_)) => return Exchange::Refused(e),
            Err(e) => return Exchange::Unanswered(e.to_string()),
        };
        for op in &walked {
            if let Err(e) = self.store_governance_op(community, op).await {
                tracing::warn!(community, error = %e, "bootstrap op not stored");
                return Exchange::Merged;
            }
        }
        rekindle_governance_runtime::rebuild_governance(self, community).await;
        Exchange::Merged
    }

    /// Our held `MemberJoined` of `community` whose hash is `hash`.
    pub(crate) async fn held_joined(&self, community: &str, hash: &[u8; 32]) -> Option<HeldOp> {
        use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
        let held = self.governance_ops(community).await.ok()?;
        held.into_iter().find(|op| op.hash.0 == *hash)
    }

    /// Queue the exchange of `joined` with the inviter at `inviter_record`
    /// as a `bootstrap` outbox row, tried until a member answers.
    pub(crate) async fn queue_bootstrap(
        &self,
        community: &str,
        inviter_record: &str,
        joined: &HeldOp,
    ) {
        let Some(bootstraps) = self.ctx.outbox.bootstraps() else {
            tracing::warn!(
                community,
                "no outbox: the bootstrap exchange is not retried"
            );
            return;
        };
        if let Err(error) = bootstraps
            .queue(community, inviter_record, &joined.hash.0)
            .await
        {
            tracing::warn!(community, %error, "bootstrap exchange not queued");
        }
    }

    /// The `bootstrap` row of `community`, discarded: we left it.
    pub(crate) fn discard_bootstrap(&self, community: &str) {
        use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
        let Some(bootstraps) = self.ctx.outbox.bootstraps() else {
            return;
        };
        let community = community.to_owned();
        self.scope()
            .spawn_or_drop("bootstrap exchange discarded", async move {
                if let Err(error) = bootstraps.discard(&community).await {
                    tracing::warn!(community = %community, %error, "bootstrap exchange not discarded");
                }
            });
    }
}
