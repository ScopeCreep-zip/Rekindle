//! Reads over session and runtime state.

use std::collections::HashSet;

use rekindle_presence::deps::{PresenceCredentials, SelfPresenceSnapshot};

use super::DaemonPresenceAdapter;

impl DaemonPresenceAdapter {
    /// Our pseudonym for a community, or empty when we are not a member.
    ///
    /// The trait returns `String` rather than `Option`, so the empty
    /// string is the "not a member" signal — callers compare against it.
    pub(super) fn my_pseudonym_impl(&self, community_id: &str) -> String {
        self.ctx.my_pseudonym(community_id)
    }

    pub(super) fn identity_display_name_impl(&self) -> String {
        self.ctx.identity_display_name()
    }

    pub(super) fn our_route_blob_impl(&self) -> Option<Vec<u8>> {
        let node = self.transport()?;
        let blob = node.personal_route_blob()?;
        // An allocated-but-empty route is not publishable: a peer that
        // imported it would have nothing to send to.
        (!blob.is_empty()).then_some(blob)
    }

    /// The daemon has no tray or status selector, so presence is
    /// "online" whenever it is running. `presence::set_status` writes
    /// the profile record directly and does not route through here.
    pub(super) fn current_status_impl() -> String {
        "online".to_string()
    }

    pub(super) fn channel_ids_impl(&self, community_id: &str) -> Vec<String> {
        self.ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(community_id))
            .map(|m| m.channel_record_keys.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub(super) fn channel_log_keys_impl(&self, community_id: &str) -> Vec<(String, String)> {
        self.ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(community_id))
            .map(|m| {
                m.channel_record_keys
                    .iter()
                    .map(|(channel, key)| (channel.clone(), key.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Members the last poll observed.
    ///
    /// This is the read that replaces `read_member_index` — one local
    /// map lookup against a roster whose every row was signature-checked
    /// when it was scanned, rather than a DHT read of a shared structure
    /// any member could have written.
    /// The writer index for the channel records: every member the roster
    /// placed in the registry, and our own slot (plan C7.12).
    pub(super) fn member_slots_impl(&self, community_id: &str) -> Vec<u32> {
        let Some(own_slot) = self
            .ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(community_id))
            .map(|m| m.slot_index)
        else {
            return Vec::new();
        };
        let mut slots: Vec<u32> = self
            .ctx
            .communities
            .members(community_id)
            .into_values()
            .map(|member| member.subkey_index)
            .collect();
        slots.push(own_slot);
        slots.sort_unstable();
        slots.dedup();
        slots
    }

    pub(super) fn presence_credentials_impl(
        &self,
        community_id: &str,
    ) -> Option<PresenceCredentials> {
        let guard = self.ctx.session.read();
        let membership = guard.as_ref()?.community(community_id)?;
        Some(PresenceCredentials {
            my_pseudonym_hex: membership.pseudonym_key.clone(),
            my_subkey_index: Some(membership.slot_index),
            // Derived on demand from the seed rather than stored: the
            // seed is the durable secret and the keypair is a pure
            // function of it plus the slot.
            slot_keypair_str: membership.slot_seed.and_then(|seed| {
                rekindle_crypto::keypair_text::slot_keypair_text(&seed, membership.slot_index).ok()
            }),
            slot_seed_hex: membership.slot_seed.map(hex::encode),
        })
    }

    /// The community's slot-registry record key, from the membership.
    pub(super) fn registry_key_impl(&self, community_id: &str) -> Option<String> {
        self.ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(community_id))
            .map(|m| m.registry_key.clone())
            .filter(|key| !key.is_empty())
    }

    /// Banned pseudonyms from merged governance — the CRDT is the
    /// authority, so this is a local read, not a DHT one.
    pub(super) fn governance_bans_impl(&self, community_id: &str) -> HashSet<String> {
        self.ctx
            .communities
            .governance_state(community_id)
            .map(|s| s.bans.iter().map(|p| hex::encode(p.0)).collect())
            .unwrap_or_default()
    }

    /// The daemon publishes no profile fields of its own — no avatar,
    /// bio, badges or RSVPs — so its presence row carries identity and
    /// liveness only. A headless node with a cosmetic profile would be
    /// inventing data no user entered.
    pub(super) fn self_presence_snapshot_impl() -> SelfPresenceSnapshot {
        SelfPresenceSnapshot::default()
    }
}
