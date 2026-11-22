//! The member-registry facts the governance runtime hands the Host (plan
//! E3.1 B2d), on both hosts: the members a create or join discovered in the
//! registry, and the registry owner keypair hydration recovers from the vault.

use std::collections::HashSet;

use rekindle_governance_runtime::deps::{DiscoveredMember, SlotRecovery};
use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};
use rekindle_vault::typed::community_keys;

use super::HostGovernance;

impl HostGovernance {
    /// Store the members a create or join read from the registry.
    ///
    /// Each pseudonym new to the community is added to its known members
    /// and announced with one `MemberDiscovered`, then every row is upserted
    /// into the identity's `community_members` in one write.
    pub(super) fn persist_discovered_registry_members_impl(
        &self,
        community_id: &str,
        members: &[DiscoveredMember],
    ) {
        let rows: Vec<_> = members
            .iter()
            .map(|m| {
                rekindle_presence::discovered_member_row(&m.presence, m.segment_index, m.slot_index)
            })
            .collect();
        let candidates = rows.iter().map(|r| r.pseudonym_key.clone()).collect();
        let new: HashSet<String> = self
            .ctx
            .communities
            .extend_known_members(community_id, candidates)
            .into_iter()
            .collect();
        // A record of a key the merge does not count is no member to
        // announce (finding I1); a member whose join merges later is
        // announced by its `Joined`.
        let governance = self.governance_state_impl(community_id).unwrap_or_default();
        let listed = super::projection::listed(&governance);
        for row in rows
            .iter()
            .filter(|r| new.contains(&r.pseudonym_key) && listed.contains(&r.pseudonym_key))
        {
            self.ctx.publish_event(SubscriptionEvent::Membership(
                MembershipEvent::MemberDiscovered {
                    community: community_id.to_string(),
                    pseudonym: row.pseudonym_key.clone(),
                    display_name: row.display_name.clone().unwrap_or_default(),
                    subkey_index: row.subkey_index,
                },
            ));
        }
        let Some(owner) = self
            .ctx
            .session
            .read()
            .as_ref()
            .map(|s| s.identity.public_key_hex.clone())
        else {
            return;
        };
        let Ok(db) = self.ctx.db.current() else {
            return;
        };
        let community = community_id.to_string();
        let joined_at = self.ctx.clock.now_secs().cast_signed();
        db.fire("persist discovered registry members", move |conn| {
            for row in &rows {
                rekindle_db::repo::members::upsert_discovered(
                    conn, &owner, &community, row, joined_at,
                )?;
            }
            Ok(())
        });
    }

    /// Restore the community's registry owner keypair from the vault, where
    /// the create stored it, when the store holds none.
    pub(super) fn recover_registry_keypair_impl(&self, community_id: &str) {
        let keypair = {
            let guard = self.ctx.vault.lock();
            let Some(vault) = guard.as_ref() else {
                return;
            };
            community_keys::load_registry_keypair(vault, community_id)
        };
        let Some(keypair) = keypair else {
            return;
        };
        tracing::info!(community = %community_id, "registry owner keypair recovered from the vault");
        self.ctx
            .communities
            .set_registry_owner_keypair(community_id, keypair);
    }
}

impl SlotRecovery for HostGovernance {
    fn list_registries_with_my_pseudonym(&self) -> Vec<(String, String, Option<String>)> {
        self.list_registries_with_my_pseudonym_impl()
    }

    fn apply_recovered_member_state(&self, community_id: &str, subkey_index: u32) {
        self.apply_recovered_member_state_impl(community_id, subkey_index);
    }

    fn try_derive_slot_keypair_if_ready(&self, community_id: &str) {
        self.try_derive_slot_keypair_impl(community_id);
    }

    fn list_missing_registry_keypairs(&self) -> Vec<String> {
        self.ctx.communities.missing_registry_keypairs()
    }

    fn recover_registry_keypair_from_keystore(&self, community_id: &str) {
        self.recover_registry_keypair_impl(community_id);
    }
}
