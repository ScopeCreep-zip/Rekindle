//! The membership control messages on both hosts (plan E3.1 B2e): the
//! `MembershipEventDeps` the governance runtime's processors
//! (`rekindle_governance_runtime::membership_events`) run over — a
//! JoinAccepted, the keypair grants, a member's roles, onboarding answers
//! and a peer-assisted join — over the Host's community store, session,
//! database and vault. `membership_route` routes the control messages here
//! and holds the background halves (peer bootstrap, presence tick).

use async_trait::async_trait;
use rekindle_governance_runtime::membership_events::{
    MemberUpsertRow, MembershipEventDeps, SlotGrantUpdate,
};
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};
use rekindle_vault::typed::community_keys;

use super::HostGovernance;

impl HostGovernance {
    /// The unlocked identity's key and database, or `None` while locked.
    pub(super) fn owner_and_db(&self) -> Option<(String, rekindle_db::Db)> {
        let owner = self
            .ctx
            .session
            .read()
            .as_ref()
            .map(|s| s.identity.public_key_hex.clone())?;
        Some((owner, self.ctx.db.current().ok()?))
    }

    /// Store one `communities` column for the community.
    fn set_community_column(
        &self,
        label: &'static str,
        community_id: &str,
        column: rekindle_db::repo::communities::Column,
        value: ColumnValue,
    ) {
        let Some((owner, db)) = self.owner_and_db() else {
            return;
        };
        let community = community_id.to_string();
        db.fire(label, move |conn| {
            use rekindle_db::repo::communities::set;
            match value {
                ColumnValue::Text(text) => set(conn, &owner, &community, column, text),
                ColumnValue::Index(index) => {
                    set(conn, &owner, &community, column, i64::from(index))
                }
            }
        });
    }
}

/// A `communities` column value a membership message sets.
enum ColumnValue {
    Text(String),
    Index(u32),
}

#[async_trait]
impl MembershipEventDeps for HostGovernance {
    fn persist_member_roles(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
        role_ids: &[u32],
        is_self: bool,
    ) {
        if is_self && !role_ids.is_empty() {
            let roles = role_ids.to_vec();
            crate::host::session::update_roles(
                &self.ctx.session,
                &self.ctx.db,
                &self.ctx.communities,
                community_id,
                move |mine| *mine = roles,
            );
        }
        let Some((owner, db)) = self.owner_and_db() else {
            return;
        };
        let (community, pseudonym, roles) = (
            community_id.to_string(),
            pseudonym_hex.to_string(),
            role_ids.to_vec(),
        );
        db.fire("member roles", move |conn| {
            rekindle_db::repo::members::set_role_ids(
                conn, &owner, &community, &pseudonym, &roles, false,
            )
        });
    }

    async fn set_mek_generation_and_registry(
        &self,
        community_id: &str,
        mek_generation: u64,
        member_registry_key: Option<&str>,
    ) {
        self.ctx.communities.set_mek_generation_and_registry(
            community_id,
            mek_generation,
            member_registry_key,
        );
        if let Some(registry) = member_registry_key {
            self.set_community_column(
                "member registry key",
                community_id,
                rekindle_db::repo::communities::Column::MemberRegistryKey,
                ColumnValue::Text(registry.to_string()),
            );
        }
    }

    async fn upsert_members(&self, community_id: &str, members: Vec<MemberUpsertRow>) {
        if members.is_empty() {
            return;
        }
        let Some((owner, db)) = self.owner_and_db() else {
            return;
        };
        let community = community_id.to_string();
        let stored = db
            .call(move |conn| {
                for m in &members {
                    rekindle_db::repo::members::replace_accepted(
                        conn,
                        &owner,
                        &community,
                        &rekindle_db::repo::members::AcceptedMember {
                            pseudonym_key: &m.pseudonym_hex,
                            display_name: &m.display_name,
                            role_ids: &m.role_ids,
                            joined_at: m.joined_at.cast_signed(),
                            subkey_index: m.subkey_index,
                            onboarding_complete: m.onboarding_complete,
                            timeout_until: m.timeout_until.map(u64::cast_signed),
                        },
                    )?;
                }
                Ok(())
            })
            .await;
        if let Err(e) = stored {
            tracing::warn!(community = %community_id, error = %e, "JoinAccepted members not stored");
        }
    }

    fn backfill_my_subkey_index(&self, community_id: &str, subkey_index: u32, persist: bool) {
        if !self
            .ctx
            .communities
            .backfill_my_subkey_index(community_id, subkey_index)
        {
            return;
        }
        tracing::info!(community = %community_id, subkey_index, "our slot index taken from the member list");
        if persist {
            self.set_community_column(
                "my subkey index from the member list",
                community_id,
                rekindle_db::repo::communities::Column::MySubkeyIndex,
                ColumnValue::Index(subkey_index),
            );
        }
    }

    fn insert_known_members(&self, community_id: &str, pseudonyms: &[String]) {
        self.ctx
            .communities
            .extend_known_members(community_id, pseudonyms.to_vec());
    }

    /// Nothing left to do: `insert_community_mek` stored the key in the
    /// vault when the cache took it (`mek::install`).
    fn persist_community_mek_to_keystore(&self, community_id: &str) {
        tracing::trace!(community = %community_id, scope = %KeyScope::Community, "community key stored at install");
    }

    fn spawn_peer_bootstrap(&self, community_id: &str, members: Vec<MemberUpsertRow>) {
        self.spawn_peer_bootstrap_impl(community_id, members);
    }

    fn apply_slot_grant(&self, community_id: &str, update: SlotGrantUpdate) {
        self.ctx.communities.apply_slot_grant(
            community_id,
            update.slot_seed_hex.as_deref(),
            update.slot_keypair.as_deref(),
            update.dht_owner_keypair.as_deref(),
            update.my_subkey_index,
        );
        {
            let guard = self.ctx.vault.lock();
            if let Some(vault) = guard.as_ref() {
                if let Some(seed) = &update.slot_seed_hex {
                    community_keys::persist_slot_seed(vault, community_id, seed);
                }
                if let Some(keypair) = &update.slot_keypair {
                    community_keys::persist_slot_keypair(vault, community_id, keypair);
                }
            }
        }
        if let Some(keypair) = update.dht_owner_keypair {
            self.set_community_column(
                "granted owner keypair",
                community_id,
                rekindle_db::repo::communities::Column::DhtOwnerKeypair,
                ColumnValue::Text(keypair),
            );
        }
        if let Some(index) = update.my_subkey_index {
            self.set_community_column(
                "granted slot index",
                community_id,
                rekindle_db::repo::communities::Column::MySubkeyIndex,
                ColumnValue::Index(index),
            );
        }
    }

    fn spawn_presence_poll_tick(&self, community_id: &str) {
        self.spawn_presence_poll_tick_impl(community_id);
    }

    fn persist_onboarding_completion(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
        role_ids: &[u32],
        is_self: bool,
    ) {
        self.ctx.communities.note_onboarding_completion(
            community_id,
            pseudonym_hex,
            role_ids,
            is_self,
        );
        let Some((owner, db)) = self.owner_and_db() else {
            return;
        };
        let (community, pseudonym, roles) = (
            community_id.to_string(),
            pseudonym_hex.to_string(),
            role_ids.to_vec(),
        );
        db.fire("onboarding completion", move |conn| {
            rekindle_db::repo::members::set_role_ids(
                conn, &owner, &community, &pseudonym, &roles, true,
            )
        });
    }

    fn push_onboarding_complete_to_sync(&self, community_id: &str) {
        if let Some(shell) = &self.ctx.shell {
            shell.push_onboarding_complete_to_sync(community_id);
        } else {
            tracing::debug!(
                community_id,
                "onboarding completion: no personal sync record on the daemon track (E5.2)"
            );
        }
    }

    fn bump_invite_uses(&self, community_id: &str, invite_code: &str) {
        let Some((owner, db)) = self.owner_and_db() else {
            return;
        };
        let code_hash = rekindle_secrets::invite::hash_invite_code(invite_code);
        let community = community_id.to_string();
        let ctx = std::sync::Arc::clone(&self.ctx);
        rekindle_governance_runtime::GovernanceRuntimeDeps::scope(self).spawn_or_drop("invite uses", async move {
            let hash = code_hash.clone();
            let cid = community.clone();
            let bumped = db
                .call(move |conn| {
                    rekindle_db::repo::community_invites::bump_uses(conn, &owner, &cid, &hash)
                })
                .await;
            match bumped {
                Ok(Some(uses)) => ctx.publish_event(SubscriptionEvent::Governance(
                    GovernanceEvent::InviteUsed {
                        community,
                        code_hash,
                        uses,
                    },
                )),
                Ok(None) => {}
                Err(e) => tracing::warn!(community = %community, error = %e, "invite uses not counted"),
            }
        });
    }
}
