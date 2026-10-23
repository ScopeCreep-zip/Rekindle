//! Joining a community from an invite on the Host (plan E3.1 D2), both
//! modes: `rekindle_governance_runtime::join_flow` claims our slot, then
//! the membership is stored and the op set merged into the state and
//! the projection, with our own member row, and the mesh hears
//! the join. Nothing petitions anyone: admission is self-sovereign.

use rekindle_types::dto::community::CommunityJoined;
use rekindle_types::invite::InviteLink;

use super::HostGovernance;

/// A joiner's roles before governance grants any: none.
const JOINER_ROLE_IDS: [u32; 0] = [];

impl HostGovernance {
    /// Join the community `link` opens.
    ///
    /// # Errors
    /// The join stages failed (an expired, banned or full invite, the DHT),
    /// or storing the membership failed.
    pub(crate) async fn join(&self, link: &InviteLink) -> Result<CommunityJoined, String> {
        let id = link.anchor.as_str().to_owned();
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
            role_ids: Vec::new(),
            slot_index: outcome.claimed.local_subkey,
            registry_key: outcome.claimed.registry_key.clone(),
            community_name: outcome.invite.community_name.clone(),
            slot_seed,
            channel_record_keys: std::collections::HashMap::new(),
            segment_index: Some(outcome.claimed.segment_index),
            lamport_counter: 0,
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
                genesis_cert: Some(outcome.cert_bytes.clone()),
                joined_at,
            },
        )
        .map_err(|e| format!("community joined but not recorded: {e}"))?;
        // `K_M(p)` and the row are durable: the walked ops and our
        // `MemberJoined` into the op store, the merge, then ours published
        // (its sender appends it to `M`/2 and writes `M`/3).
        self.store_join_ops(&id, &outcome).await?;
        rekindle_governance_runtime::rebuild_governance(self, &id).await;
        rekindle_governance_runtime::deps::GovernanceRuntimeDeps::publish_governance_op(
            self,
            &id,
            outcome.joined.encoded.clone(),
        )
        .await
        .map_err(|e| format!("community joined, MemberJoined not published: {e}"))?;
        self.store_own_member_row(&id, &pseudonym, joined_at)
            .await?;
        self.announce_join(&id);
        // Our `MemberJoined` is durable on `M`: tell the inviter (it
        // verifies and merges it), then continue the walk from its frontier.
        self.bootstrap_from_inviter(&id, &outcome).await;
        // The invite carries no community key (D11): ask for it.
        self.recover_behind_meks();
        self.audit_joined(&id, &outcome.invite.community_name);
        tracing::info!(community = %id, "joined community");
        Ok(CommunityJoined {
            governance_key: id,
            name: outcome.invite.community_name.clone(),
            segment: outcome.claimed.segment_index,
            slot: outcome.claimed.local_subkey,
            known_members: u32::try_from(outcome.initial_presence.known_members.len())
                .unwrap_or(u32::MAX),
        })
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
                &JOINER_ROLE_IDS,
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
