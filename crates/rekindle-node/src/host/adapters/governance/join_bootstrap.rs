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
use rekindle_types::join_refusal::JoinRefusal;

use super::HostGovernance;

/// Why a join did not run (ADR 0019's outcome rule): the inviter's typed
/// refusal, which the frontends render as told, or a failure.
#[derive(Debug)]
pub(crate) enum JoinFailure {
    Refused(JoinRefusal),
    Failed(String),
}

impl From<String> for JoinFailure {
    fn from(e: String) -> Self {
        Self::Failed(e)
    }
}

/// How one bootstrap exchange ended.
pub(crate) enum Exchange {
    /// A member answered our `MemberJoined` with its frontier; the ops our
    /// walk from it found are stored and the community rebuilt.
    Merged,
    /// The inviter answered and refused it, and why: every member's merge
    /// holds it invalid.
    Refused(JoinRefusal),
    /// Nobody answered (the inviter is away, holds no route, answered
    /// `RETRY_LATER` past its deadline, or was partial and no member
    /// answered): the join stands from the DHT and the exchange is tried
    /// again.
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
    /// `MemberJoined` `refused_hash`: withdraw the join as the join itself
    /// does when it is refused at once (our `MemberLeft`, the community
    /// forgotten), never a membership no member admits.
    ///
    /// Under the community's join lock, as a join runs: a re-join waits for
    /// the withdrawal, and a withdrawal after a re-join finds its
    /// `MemberJoined` superseded and leaves nothing (the leave forgot the
    /// re-joined community under the re-join's feet).
    pub(crate) async fn withdraw_refused_join(
        &self,
        community: &str,
        refused_hash: &[u8; 32],
        refused: &JoinRefusal,
    ) {
        let lock = self.ctx.community_joins.of(community);
        let _joining = lock.lock().await;
        if !self.holds_community(community) {
            return;
        }
        let mine = (self.ctx.communities.map().read())
            .get(community)
            .and_then(|c| {
                c.my_pseudonym_key
                    .as_deref()
                    .and_then(super::ops::pseudonym_of)
            });
        let Some(mine) = mine else {
            return;
        };
        if self
            .latest_own_joined(community, &mine)
            .await
            .map(|op| op.hash().0)
            != Some(*refused_hash)
        {
            tracing::info!(
                community,
                "refused join superseded by a later one: not withdrawn"
            );
            return;
        }
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
    /// verify. The read's seq goes to the transport's dead-route rule (plan
    /// RV1g B3).
    pub(crate) async fn inviter_route(
        &self,
        community: &str,
        inviter_record: &str,
    ) -> Option<Vec<u8>> {
        use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
        let inviter = self.inviter_of(community, inviter_record)?;
        let owner = self
            .ctx
            .communities
            .governance_state(community)?
            .members
            .get(&inviter)?
            .member_record_owner
            .clone();
        let port = self.record_port().ok()?;
        let read = rekindle_governance_runtime::member_routes::read_member_route_at(
            &*port,
            community,
            inviter_record,
            &inviter,
            rekindle_governance_runtime::member_routes::RoutePin::Joined(&owner),
        )
        .await;
        match read {
            Ok(Some((seq, route))) => {
                self.route_read_at(&route, seq);
                Some(route).filter(|r| !r.is_empty())
            }
            Ok(None) => None,
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
        use rekindle_governance_runtime::deps::GovernanceOpStore as _;
        if inviter_route.is_empty() {
            return Exchange::Unanswered("the inviter holds no route".into());
        }
        let joiner = BootstrapJoiner {
            pseudonym: hex::encode(joined.author.0),
            member_joined: OpRef {
                op_hash: joined.hash,
                device_key: joined.device_key,
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
            Err(GovernanceRuntimeError::BootstrapRefused(reason)) => {
                tracing::info!(
                    community,
                    source = "BootstrapRefused",
                    ?reason,
                    "join refused"
                );
                return Exchange::Refused(reason);
            }
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
        use rekindle_governance_runtime::deps::GovernanceOpStore as _;
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
