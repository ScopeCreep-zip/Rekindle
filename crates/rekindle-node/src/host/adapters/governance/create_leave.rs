//! Creating and leaving a community on the Host (plan E3.1 D1b), both
//! modes. Creation runs `rekindle_governance_runtime::create_community`
//! and stores what the window reads back: the projected channels, roles and
//! categories, and our own member row. Leaving stops the community's tasks,
//! releases our registry slot (`operations::community::leave_community`),
//! tells the mesh, returns the records to the pool and forgets every key,
//! row and store entry of the community. The departure is what triggers
//! the remaining members' key rotation (`community_rpc/leave.rs` until
//! E3.5).

use rekindle_types::channel_keys::KeyScope;
use rekindle_types::dto::community::CommunityCreated;
use rekindle_types::governance::{AdmissionMode, GovernanceEntry};
use rekindle_types::id::ChannelId;

use super::HostGovernance;

/// Why a create is refused before it starts: Veilid's DHT is online only
/// while the public internet is ready (0.5.8 `storage_manager/mod.rs:1096-1104`),
/// and a create cannot be held for later (the anchor's one-time key, ADR
/// 0011), so it waits for the network rather than queueing.
pub(crate) const NOT_CONNECTED: &str = "Not connected to the network yet; try again when connected";

/// How long a left community's tasks get to stop.
const COMMUNITY_STOP_DEADLINE: std::time::Duration = rekindle_types::config::SESSION_STOP_DEADLINE;

impl HostGovernance {
    /// Create `name` with the genesis `admission`; `description` is written
    /// as metadata after genesis.
    ///
    /// # Errors
    /// The creation, or storing its rows, failed.
    pub(crate) async fn create(
        &self,
        name: &str,
        description: &str,
        admission: AdmissionMode,
    ) -> Result<CommunityCreated, String> {
        if !self.transport().is_ok_and(|node| node.is_ready()) {
            return Err(NOT_CONNECTED.into());
        }
        let id = rekindle_governance_runtime::create_community(self, name, admission)
            .await
            .map_err(|e| format!("community create failed: {e}"))?;
        if !description.is_empty() {
            rekindle_governance_runtime::apply::write_entry(
                self,
                &id,
                GovernanceEntry::CommunityMeta {
                    name: Some(name.to_owned()),
                    description: Some(description.to_owned()),
                    icon_hash: None,
                    banner_hash: None,
                },
            )
            .await
            .map_err(|e| format!("community created, description not written: {e}"))?;
        }
        self.persist_governance_snapshot(&id).await;
        let pseudonym = self
            .ctx
            .communities
            .map()
            .read()
            .get(&id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .ok_or("community not found after creation")?;
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "no identity is unlocked")?;
        let display_name = self.ctx.identity_display_name();
        let joined_at = self.ctx.clock.now_ms_i64();
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let cid = id.clone();
        db.call(move |conn| {
            rekindle_db::repo::members::insert_if_absent(
                conn,
                &owner,
                &cid,
                &pseudonym,
                &display_name,
                joined_at,
            )
        })
        .await
        .map_err(|e| format!("community created, member row not stored: {e}"))?;
        let registry_key = self
            .ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(&id))
            .map(|m| m.registry_key.clone())
            .unwrap_or_default();
        Ok(CommunityCreated {
            anchor: id,
            registry_key,
            name: name.to_owned(),
        })
    }

    /// Leave `membership`'s community.
    ///
    /// A joiner whose own `MemberJoined` our merged state never admitted
    /// (it lost a single-use race, or the inviter refused it) withdraws
    /// locally only (plan SU1, the withdrawal decision): there is nothing
    /// on the DAG to undo and the join can never become valid (ADR 0011:
    /// membership is the merged state's alone), so it authors no
    /// `MemberLeft`, no `Departed` beat and no departure notice, and drops
    /// the community's tasks, keys, leases, rows and queued ops.
    ///
    /// # Errors
    /// The transport is down, no identity is unlocked, or forgetting the
    /// community failed.
    pub(crate) async fn leave(
        &self,
        membership: &rekindle_transport::CommunityMembership,
    ) -> Result<(), String> {
        let id = membership.anchor.clone();
        let transport = self.transport().map_err(|e| e.to_string())?;
        if !self.own_join_admitted(&id) {
            tracing::warn!(
                community = %id,
                "our join is not admitted by our merged state: withdrawn locally, nothing authored"
            );
            self.stop_community_tasks(&id).await;
            self.ctx.mek_cache.write().remove_community(&id);
            self.discard_governance_ops(&id);
            return self.forget_community(&transport, &id).await;
        }
        // The `MemberLeft` op goes on our member record first, while the
        // community's state and our pseudonym are still here (plan E3.2).
        rekindle_governance_runtime::moderation::leave(self, &id)
            .await
            .map_err(|e| format!("community leave op: {e}"))?;
        // Then our `Departed` beat, which ends the community's beats.
        crate::host::adapters::presence::depart(&self.ctx, &id).await;
        let signing_key = self
            .ctx
            .signing_key
            .read()
            .as_ref()
            .map(|k| zeroize::Zeroizing::new(*k.as_bytes()))
            .ok_or("no identity is unlocked")?;
        self.stop_community_tasks(&id).await;
        let left = rekindle_transport::operations::community::leave_community(
            &transport,
            membership,
            &self.ctx.mek_cache,
            &signing_key,
            self.ctx.clock.now_secs(),
        )
        .await
        .map_err(|e| format!("community leave failed: {e}"))?;
        self.send_before_teardown(&id, &left.departure_notice).await;
        self.forget_community(&transport, &id).await
    }

    /// Whether our own join is in our merged state: the cert names us (the
    /// creator) or the merge admitted our `MemberJoined`, kept after we
    /// leave or are removed (`op_log::author_record`).
    fn own_join_admitted(&self, community: &str) -> bool {
        let map = self.ctx.communities.map().read();
        let Some(c) = map.get(community) else {
            return false;
        };
        let mine = c
            .my_pseudonym_key
            .as_deref()
            .and_then(super::ops::pseudonym_of);
        let (Some(cert), Some(mine)) = (c.genesis_cert.as_ref(), mine) else {
            return false;
        };
        let state = c.governance_state.clone().unwrap_or_default();
        rekindle_governance_runtime::op_log::author_record(cert, &state, &mine).is_some()
    }

    /// Stop the community's loops, so none writes to a record the leave
    /// clears or closes.
    async fn stop_community_tasks(&self, id: &str) {
        let daemon_tasks = self.ctx.community_scopes.lock().remove(id);
        let store_tasks = self
            .ctx
            .communities
            .map()
            .read()
            .get(id)
            .and_then(|c| c.tasks.clone());
        for tasks in [daemon_tasks, store_tasks].into_iter().flatten() {
            if let Err(stuck) = tasks.shutdown(COMMUNITY_STOP_DEADLINE).await {
                tracing::warn!(community = %id, %stuck, "community tasks did not stop in time");
            }
        }
    }

    /// Our withdrawn join's queued ops (its `MemberJoined`) go with it.
    fn discard_governance_ops(&self, community: &str) {
        use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
        let Some(ops) = self.ctx.outbox.governance_ops() else {
            return;
        };
        let community = community.to_owned();
        self.scope()
            .spawn_or_drop("governance ops discarded", async move {
                if let Err(error) = ops.discard_community(&community).await {
                    tracing::warn!(community = %community, %error, "governance ops not discarded");
                }
            });
    }

    /// Forget the community locally: its watch ring, leases, keys, queued
    /// rows, store entry and stored membership.
    async fn forget_community(
        &self,
        transport: &rekindle_transport::TransportNode,
        id: &str,
    ) -> Result<(), String> {
        // A follow-up a rebuild scheduled after the loops stopped started
        // on a fresh community scope: stop it before the ring is released.
        let late_tasks = self.ctx.community_scopes.lock().remove(id);
        if let Some(tasks) = late_tasks {
            if let Err(stuck) = tasks.shutdown(COMMUNITY_STOP_DEADLINE).await {
                tracing::warn!(community = %id, %stuck, "community tasks did not stop in time");
            }
        }
        self.release_watch_ring(id).await;
        let (leases, ring_leases) = self.ctx.communities.take_leases(id);
        if let Some(subs) = self.ctx.subscriptions.read().as_ref() {
            subs.teardown_community(id);
        }
        for lease in leases.all().chain(ring_leases) {
            rekindle_transport::broadcast::dht_writes::release(transport, lease).await;
        }
        self.forget_keys(id);
        self.discard_key_transfers(id, None);
        self.discard_bootstrap(id);
        crate::host::adapters::channel::discard_waiting_posts(self, id);
        self.ctx.communities.remove(id);
        crate::host::session::forget_membership(
            &self.ctx.session,
            &self.ctx.db,
            &self.ctx.vault,
            id,
        )
        .await
        .map_err(|e| format!("community left but not forgotten: {e}"))?;
        self.audit_left(id);
        tracing::info!(community = %id, "left community");
        Ok(())
    }

    /// Gossip our departure (or our quit's hint) to the mesh before the
    /// teardown, every peer's send awaited
    /// ([`Self::send_to_mesh_awaited`]).
    pub(crate) async fn send_before_teardown(
        &self,
        community: &str,
        notice: &rekindle_codec::community::envelope::CommunityEnvelope,
    ) {
        match self.send_to_mesh_awaited(community, notice).await {
            Ok(peers) => tracing::debug!(peers, "gossip before teardown sent"),
            Err(error) => tracing::warn!(%error, "gossip before teardown not sent"),
        }
    }

    /// Every key of every scope of `community`, stored and live, ours and
    /// the shell's: a member who left keeps nothing that opens the
    /// community's past or future traffic.
    pub(super) fn forget_keys(&self, community: &str) {
        let mut scopes = vec![KeyScope::Community];
        if let Some(c) = self.ctx.communities.map().read().get(community) {
            scopes.extend(
                c.channels
                    .iter()
                    .filter_map(|ch| ChannelId::from_hex(&ch.id).map(KeyScope::Channel)),
            );
        }
        if let Some(vault) = self.ctx.vault.lock().as_ref() {
            for scope in scopes {
                rekindle_vault::typed::mek::delete_scope_meks(vault, community, scope);
            }
            let slot = rekindle_vault::VaultKey::SlotKeypair {
                community: community.to_owned(),
            };
            if let Err(e) = vault.delete(&slot) {
                tracing::warn!(community, error = %e, "slot keypair not deleted");
            }
            rekindle_vault::typed::community_keys::delete_member_hpke_secret(vault, community);
        }
        if let Some(shell) = &self.ctx.shell {
            shell.community_left(community);
        }
    }

    fn audit_left(&self, community: &str) {
        let (Ok(owner), Ok(db)) = (self.ctx.require_owner(), self.ctx.db.current()) else {
            return;
        };
        crate::host::audit::append(
            &self.ctx.audit_chain,
            &db,
            &self.ctx.vault,
            &owner,
            rekindle_audit::AuditKind::ChannelLeft,
            serde_json::json!({ "community_id": community }),
            self.ctx.clock.now_ms_i64(),
        );
    }
}

/// Why a Forget was refused or failed.
#[derive(Debug)]
pub(crate) enum ForgetError {
    /// We were not removed: a member leaves instead (a Forget writes no op).
    NotRemoved,
    Failed(String),
}

impl HostGovernance {
    /// Forget a community we were removed from (plan E3.2 S9 R3-5, Element's
    /// "Forget this room", Signal's delete after "removed you"): its kept
    /// history, keys, rows and store entry go. No op is written and no one
    /// is told: the removal already took us out, and the community was torn
    /// down when it was seen (`own_removal.rs`).
    ///
    /// # Errors
    /// [`ForgetError::NotRemoved`] for a community we still belong to;
    /// otherwise forgetting its rows failed.
    pub(crate) async fn forget(&self, community: &str) -> Result<(), ForgetError> {
        let gate = self
            .ctx
            .communities
            .removal_gate(community)
            .ok_or(ForgetError::NotRemoved)?;
        let _torn_down = gate.lock().await;
        if self.ctx.communities.own_removal(community).is_none() {
            return Err(ForgetError::NotRemoved);
        }
        self.forget_keys(community);
        self.ctx.mek_cache.write().remove_community(community);
        self.ctx.communities.remove(community);
        crate::host::session::forget_membership(
            &self.ctx.session,
            &self.ctx.db,
            &self.ctx.vault,
            community,
        )
        .await
        .map_err(|e| ForgetError::Failed(format!("community not forgotten: {e}")))?;
        self.audit_left(community);
        tracing::info!(community = %community, "removed community forgotten");
        Ok(())
    }

    /// Undo a create that failed after origin installed the community
    /// (its deadline, or a later write): its keys, MEK, store entry and
    /// rows go. Its records were already released; no one knew of it.
    pub(crate) async fn discard_created(&self, community: &str) {
        self.forget_keys(community);
        self.ctx.mek_cache.write().remove_community(community);
        self.ctx.communities.remove(community);
        if let Err(e) = crate::host::session::forget_membership(
            &self.ctx.session,
            &self.ctx.db,
            &self.ctx.vault,
            community,
        )
        .await
        {
            tracing::warn!(error = %e, "an unfinished community's rows were not removed");
        }
    }
}
