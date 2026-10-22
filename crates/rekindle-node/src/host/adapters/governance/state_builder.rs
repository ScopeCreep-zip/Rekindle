//! A created community's runtime state (plan E3.1 B2e), on both hosts: what
//! the origin flow's `CommunityInsert` gives the store beyond the recorded
//! membership — the creator's slot keypair, the genesis channel's record,
//! the open records, the creator as a known member and an empty gossip
//! overlay. Channels, roles and metadata come from the genesis governance
//! (`projection`).

use rekindle_vault::typed::community_keys;

use super::HostGovernance;
use crate::host::community::GossipOverlay;

/// The parts of a `CommunityInsert` the store keeps at creation.
pub(super) struct CreatedCommunity<'a> {
    pub id: &'a str,
    pub governance_key: &'a str,
    pub registry_key: &'a str,
    pub channel_id_hex: &'a str,
    pub channel_record_key: &'a str,
    pub slot_keypair: &'a str,
    pub my_pseudonym_hex: &'a str,
}

impl HostGovernance {
    /// Fill the created community's store entry and keep the creator's slot
    /// keypair in the vault.
    pub(super) fn install_created(&self, created: &CreatedCommunity<'_>) {
        {
            let mut map = self.ctx.communities.map().write();
            let Some(c) = map.get_mut(created.id) else {
                return;
            };
            c.slot_keypair = Some(created.slot_keypair.to_string());
            c.channel_log_keys.insert(
                created.channel_id_hex.to_string(),
                created.channel_record_key.to_string(),
            );
            c.open_community_records.governance_key = Some(created.governance_key.to_string());
            c.open_community_records.registry_key = Some(created.registry_key.to_string());
            c.open_community_records
                .channel_keys
                .push(created.channel_record_key.to_string());
            c.known_members.insert(created.my_pseudonym_hex.to_string());
            c.gossip.get_or_insert_with(GossipOverlay::default);
            c.onboarding_complete = true;
        }
        let guard = self.ctx.vault.lock();
        if let Some(vault) = guard.as_ref() {
            community_keys::persist_slot_keypair(vault, created.id, created.slot_keypair);
        }
    }
}
