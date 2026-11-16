//! Joining a community from an invite on the Host (plan E3.1 D2), both
//! modes: `rekindle_governance_runtime::join_flow` claims our slot, then
//! the membership is stored and the op set merged into the state and
//! the projection, with our own member row, and the mesh hears
//! the join. Nothing petitions anyone: admission is self-sovereign.

use rekindle_types::dto::community::CommunityJoined;
use rekindle_types::invite::InviteLink;

use super::HostGovernance;

/// One lock per community a join is in flight for (review RV7 M3): a
/// second join of it waits for the first and answers its membership.
#[derive(Default)]
pub(crate) struct JoinLocks(
    parking_lot::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
);

impl JoinLocks {
    fn of(&self, community: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        std::sync::Arc::clone(self.0.lock().entry(community.to_owned()).or_default())
    }
}

impl HostGovernance {
    /// Join the community `link` opens.
    ///
    /// # Errors
    /// The join stages failed (an expired, banned or full invite, the DHT),
    /// or storing the membership failed.
    pub(crate) async fn join(&self, link: &InviteLink) -> Result<CommunityJoined, String> {
        let id = link.anchor.as_str().to_owned();
        // Idempotent (RV4 review M1, RV7 M3): a join answered `Pending`,
        // repeated or concurrent (the lock holds the second until the first
        // is recorded) finds us a current member and answers it, and never
        // mints a second member record or `MemberJoined`. A member kicked or
        // banned is no current member, so it joins again.
        let lock = self.ctx.community_joins.of(&id);
        let _joining = lock.lock().await;
        if let Some(joined) = self.already_joined(&id).await {
            return Ok(joined);
        }
        let outcome = rekindle_governance_runtime::join_flow::run_join_stages(self, link)
            .await
            .map_err(|e| format!("community join failed: {e}"))?;
        let slot_seed = hex::decode(&outcome.invite.slot_seed)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b).ok());
        let pseudonym = outcome.identity.pseudo_hex.clone();
        let membership = rekindle_transport::session::CommunityMembership {
            governance_key: id.clone(),
            pseudonym_key: pseudonym.clone(),
            slot_index: outcome.claimed.local_subkey,
            registry_key: outcome.claimed.registry_key.clone(),
            community_name: outcome.invite.community_name.clone(),
            slot_seed,
            channel_record_keys: std::collections::HashMap::new(),
            segment_index: Some(outcome.claimed.segment_index),
            lamport_counter: 0,
            // No key yet: 0 is the sentinel for "no community key held",
            // never a generation (the genesis key is
            // `GENESIS_MEK_GENERATION`, 1). The key arrives through
            // `recover_behind_meks`; any "behind" test is
            // `local < governance.mek_generation` (review RV7 L1).
            mek_generation: 0,
        };
        let joined_at = self.ctx.clock.now_ms_i64();
        crate::host::session::record_membership(
            &self.ctx.session,
            &self.ctx.db,
            &self.ctx.vault,
            &self.ctx.communities,
            crate::host::session::NewMembership {
                membership,
                dht_owner_keypair: None,
                registry_keypair: None,
                member_record_owner: Some(outcome.member.owner_keypair.clone()),
                member_hpke_secret: Some(outcome.member.hpke_secret.clone()),
                genesis_cert: Some(outcome.cert_bytes.clone()),
                joined_at,
            },
        )
        .map_err(|e| format!("community joined but not recorded: {e}"))?;
        self.open_gossip_mesh(&id);
        // `K_M(p)`, the card's HPKE secret and the row are durable: the
        // walked ops and our `MemberJoined` into the op store, the merge,
        // then `M` in §4.4's order.
        self.store_join_ops(&id, &outcome).await?;
        rekindle_governance_runtime::rebuild_governance(self, &id).await;
        let joined_queued = self
            .publish_joined_record(&id, &outcome)
            .await
            .map_err(|e| format!("community joined, member record not published: {e}"))?;
        self.store_own_member_row(&id, &pseudonym, joined_at)
            .await?;
        self.announce_join(&id);
        // Our `MemberJoined` is on `M` (or queued there and gossiped to the
        // mesh): tell the inviter (it verifies and merges it), then continue
        // the walk from its frontier. The invite carries no community key
        // (D11): the inviter pushes it by `MekTransfer` once it merged our
        // `MemberJoined` (D16).
        self.bootstrap_from_inviter(&id, &outcome).await;
        self.audit_joined(&id, &outcome.invite.community_name);
        tracing::info!(community = %id, "joined community");
        // Our `MemberJoined` not on `M` yet is a pending success: applied,
        // gossiped and queued, and the outbox lands it (S6b).
        Ok(CommunityJoined {
            governance_key: id,
            name: outcome.invite.community_name.clone(),
            segment: outcome.claimed.segment_index,
            slot: outcome.claimed.local_subkey,
            known_members: u32::try_from(outcome.initial_presence.known_members.len())
                .unwrap_or(u32::MAX),
            pending: joined_queued,
        })
    }

    /// Our membership of `id` when the merged state shows us a current
    /// member (in `members`, not `removed` or banned): its stored state,
    /// pending while our own `MemberJoined` is still queued on the
    /// governance-op stream (review RV7 M3).
    async fn already_joined(&self, id: &str) -> Option<CommunityJoined> {
        let (joined, mine) = {
            let map = self.ctx.communities.map().read();
            let community = map.get(id)?;
            let gov = community.governance_state.as_ref()?;
            let mine = community
                .my_pseudonym_key
                .as_deref()
                .and_then(super::ops::pseudonym_of)?;
            let current = gov.members.contains_key(&mine)
                && !gov.removed.contains_key(&mine)
                && !gov.bans.contains(&mine);
            if !current {
                return None;
            }
            let joined = CommunityJoined {
                governance_key: id.to_owned(),
                name: community.name.clone(),
                segment: community.my_segment_index.unwrap_or_default(),
                slot: community.my_subkey_index.unwrap_or_default(),
                known_members: u32::try_from(community.members.len()).unwrap_or(u32::MAX),
                pending: false,
            };
            (joined, mine)
        };
        let pending = self.member_joined_queued(id, &mine).await;
        Some(CommunityJoined { pending, ..joined })
    }

    /// Whether our own latest `MemberJoined` of `id` is still queued.
    async fn member_joined_queued(
        &self,
        id: &str,
        mine: &rekindle_types::id::PseudonymKey,
    ) -> bool {
        let Ok(held) = self.governance_ops_impl(id).await else {
            return false;
        };
        let set = rekindle_governance_runtime::op_log::verify_held(id, &held);
        let Some(op) = set
            .ops
            .iter()
            .filter(|op| {
                op.author() == mine
                    && matches!(
                        op.body(),
                        rekindle_types::governance::GovernanceEntry::MemberJoined { .. }
                    )
            })
            .max_by_key(|op| op.lamport())
        else {
            return false;
        };
        let Some(ops) = self.ctx.outbox.governance_ops() else {
            return false;
        };
        ops.is_queued(&hex::encode(op.hash().0))
            .await
            .unwrap_or(false)
    }

    /// Our `M` in §4.4's order: the card (`M`/0), our `MemberJoined` on
    /// `M`/2 with `M`/3 after it (the governance-op stream's send), then
    /// the beat (`M`/1). `true`: the `MemberJoined` is pending (its first
    /// attempt did not land within `GOVERNANCE_ANSWER_BOUND`) and stays
    /// queued; the beat is written regardless.
    async fn publish_joined_record(
        &self,
        community: &str,
        outcome: &rekindle_governance_runtime::join_flow::JoinStagesOutcome,
    ) -> Result<bool, String> {
        use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
        use rekindle_governance_runtime::member_card::{publish_own_record, OwnProfile, OwnRecord};
        let port = self.record_port().map_err(|e| e.to_string())?;
        let own = OwnRecord {
            community_id: community,
            record: &outcome.member.record_key,
            writer: &outcome.member.owner_keypair,
            pseudonym: &outcome.identity.pseudonym_signing,
        };
        let profile = OwnProfile {
            display_name: self.ctx.identity_display_name(),
            status: self.identity_status(),
            now_ms: self.ctx.clock.now_ms(),
        };
        let queued = std::sync::atomic::AtomicBool::new(false);
        let publish_ops = async {
            match self
                .publish_governance_op(
                    community,
                    outcome.joined.encoded.clone(),
                    rekindle_governance_runtime::OpLanding::FirstAttempt,
                )
                .await
            {
                Ok(rekindle_governance_runtime::OpOutcome::Landed) => Ok(()),
                Ok(rekindle_governance_runtime::OpOutcome::Pending) => {
                    queued.store(true, std::sync::atomic::Ordering::Relaxed);
                    Ok(())
                }
                Err(e) => Err(format!("MemberJoined not published: {e}")),
            }
        };
        publish_own_record(
            &*port,
            &own,
            &profile,
            &outcome.member.hpke_secret,
            publish_ops,
        )
        .await?;
        Ok(queued.load(std::sync::atomic::Ordering::Relaxed))
    }

    async fn store_join_ops(
        &self,
        community: &str,
        outcome: &rekindle_governance_runtime::join_flow::JoinStagesOutcome,
    ) -> Result<(), String> {
        use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
        for op in outcome
            .walked
            .iter()
            .chain(std::iter::once(&outcome.joined))
        {
            self.store_governance_op(community, op)
                .await
                .map_err(|e| format!("community joined, op not stored: {e}"))?;
        }
        Ok(())
    }

    /// The bootstrap exchange (plan E3.2). The join already stands from
    /// the DHT; an inviter that is not online leaves it as it is.
    async fn bootstrap_from_inviter(
        &self,
        community: &str,
        outcome: &rekindle_governance_runtime::join_flow::JoinStagesOutcome,
    ) {
        use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
        let joined = &outcome.joined;
        let joiner = rekindle_governance_runtime::BootstrapJoiner {
            pseudonym: outcome.identity.pseudo_hex.clone(),
            member_joined: rekindle_types::governance::OpRef {
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
            &outcome.invite.inviter_route_blob,
            community,
            &joiner,
        )
        .await
        {
            Ok(walked) => walked,
            Err(e) => {
                tracing::info!(community, error = %e, "bootstrap exchange not completed");
                return;
            }
        };
        for op in &walked {
            if let Err(e) = self.store_governance_op(community, op).await {
                tracing::warn!(community, error = %e, "bootstrap op not stored");
                return;
            }
        }
        rekindle_governance_runtime::rebuild_governance(self, community).await;
    }

    async fn store_own_member_row(
        &self,
        community: &str,
        pseudonym: &str,
        joined_at: i64,
    ) -> Result<(), String> {
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "no identity is unlocked")?;
        let display_name = self.ctx.identity_display_name();
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let (cid, pk) = (community.to_owned(), pseudonym.to_owned());
        db.call(move |conn| {
            rekindle_db::repo::members::insert_if_absent(
                conn,
                &owner,
                &cid,
                &pk,
                &display_name,
                joined_at,
            )
        })
        .await
        .map_err(|e| format!("community joined, member row not stored: {e}"))
    }

    fn audit_joined(&self, community: &str, name: &str) {
        let (Ok(owner), Ok(db)) = (self.ctx.require_owner(), self.ctx.db.current()) else {
            return;
        };
        crate::host::audit::append(
            &self.ctx.audit_chain,
            &db,
            &self.ctx.vault,
            &owner,
            rekindle_audit::AuditKind::ChannelJoined,
            serde_json::json!({ "community_id": community, "community_name": name }),
            self.ctx.clock.now_ms_i64(),
        );
    }
}

#[cfg(test)]
#[path = "join_tests.rs"]
mod tests;
