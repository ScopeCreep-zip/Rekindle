//! Creating and leaving a community on the Host (plan E3.1 D1b), both
//! modes. Creation runs `rekindle_governance_runtime::create_community`
//! and stores what the window reads back: the projected channels, roles and
//! categories, and our own member row. Leaving stops the community's tasks,
//! releases our registry slot (`operations::community::leave_community`),
//! tells the mesh, returns the records to the pool and forgets every key,
//! row and store entry of the community. The departure is what triggers
//! the remaining members' key rotation (`community_rpc/leave.rs` until
//! E3.5).

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::dto::community::CommunityCreated;
use rekindle_types::governance::{AdmissionMode, GovernanceEntry};
use rekindle_types::id::ChannelId;

use super::HostGovernance;

/// The creator's roles: the owner role and `@everyone`.
const CREATOR_ROLE_IDS: [u32; 2] = [0, u32::MAX];

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
                &CREATOR_ROLE_IDS,
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
            governance_key: id,
            registry_key,
            name: name.to_owned(),
        })
    }

    /// Leave `membership`'s community.
    ///
    /// # Errors
    /// The transport is down, no identity is unlocked, or forgetting the
    /// community failed.
    pub(crate) async fn leave(
        &self,
        membership: &rekindle_transport::CommunityMembership,
    ) -> Result<(), String> {
        let id = membership.governance_key.clone();
        let transport = self.transport().map_err(|e| e.to_string())?;
        let signing_key = self
            .ctx
            .signing_key
            .read()
            .as_ref()
            .map(|k| zeroize::Zeroizing::new(*k.as_bytes()))
            .ok_or("no identity is unlocked")?;
        // The loops stop first, so none writes to a record this leave
        // clears or closes.
        let daemon_tasks = self.ctx.community_scopes.lock().remove(&id);
        let store_tasks = self
            .ctx
            .communities
            .map()
            .read()
            .get(&id)
            .and_then(|c| c.tasks.clone());
        for tasks in [daemon_tasks, store_tasks].into_iter().flatten() {
            if let Err(stuck) = tasks.shutdown(COMMUNITY_STOP_DEADLINE).await {
                tracing::warn!(community = %id, %stuck, "community tasks did not stop in time");
            }
        }
        let left = rekindle_transport::operations::community::leave_community(
            &transport,
            membership,
            &self.ctx.mek_cache,
            &signing_key,
            self.ctx.clock.now_secs(),
        )
        .await
        .map_err(|e| format!("community leave failed: {e}"))?;
        self.send_departure(&id, &left.departure_notice).await;
        let leases = self.ctx.communities.take_leases(&id);
        let watch_leases = self
            .ctx
            .subscriptions
            .read()
            .as_ref()
            .map(|subs| subs.teardown_community(&id))
            .unwrap_or_default();
        for lease in leases.all().chain(watch_leases) {
            rekindle_transport::broadcast::dht_writes::release(&transport, lease).await;
        }
        self.forget_keys(&id);
        self.ctx.communities.remove(&id);
        crate::host::session::forget_membership(
            &self.ctx.session,
            &self.ctx.db,
            &self.ctx.vault,
            &id,
        )
        .await
        .map_err(|e| format!("community left but not forgotten: {e}"))?;
        self.audit_left(&id);
        tracing::info!(community = %id, "left community");
        Ok(())
    }

    /// Gossip our departure while the community's overlay still holds its
    /// peers: the daemon's worker is awaited until the fan-out has them,
    /// since the teardown that follows removes the overlay.
    async fn send_departure(
        &self,
        community: &str,
        notice: &rekindle_codec::community::envelope::CommunityEnvelope,
    ) {
        if let Some(shell) = &self.ctx.shell {
            if let Err(e) = shell.send_to_mesh(community, notice) {
                tracing::warn!(community, error = %e, "departure not gossiped");
            }
            return;
        }
        crate::daemon::gossip::send_awaited(&self.ctx.gossip_tx, community, notice).await;
    }

    /// Every key of every scope of `community`, stored and live, ours and
    /// the shell's: a member who left keeps nothing that opens the
    /// community's past or future traffic.
    fn forget_keys(&self, community: &str) {
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
