//! The member-registry facts the governance runtime hands the Host (plan
//! E3.1 B2d), on both hosts: the members a create or join discovered in the
//! registry, and the registry owner keypair hydration recovers from the vault.

use std::collections::{HashMap, HashSet};

use rekindle_governance_runtime::deps::DiscoveredMember;
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
        let roles: HashMap<String, Vec<u32>> = members
            .iter()
            .map(|m| (hex::encode(m.presence.pseudonym_key.0), m.role_ids.clone()))
            .collect();
        let rows: Vec<_> = members
            .iter()
            .map(|m| {
                rekindle_presence::discovered_member_row(
                    &m.presence,
                    m.segment_index,
                    m.slot_index,
                    &roles,
                )
            })
            .collect();
        let candidates = rows.iter().map(|r| r.pseudonym_key.clone()).collect();
        let new: HashSet<String> = self
            .ctx
            .communities
            .extend_known_members(community_id, candidates)
            .into_iter()
            .collect();
        for row in rows.iter().filter(|r| new.contains(&r.pseudonym_key)) {
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
