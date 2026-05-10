//! MEK rotation and on-demand request/transfer.
//!
//! Message Encryption Keys (MEKs) are per-channel symmetric keys used to encrypt
//! channel messages. Each MEK generation is wrapped for every community member
//! via X25519 ECDH + HKDF + AES-256-GCM and stored in the registry's MEK vault.
//!
//! Rotation: operator generates a new key, wraps for all current members, writes
//! to vault, broadcasts MekRotated gossip. Members that miss the gossip discover
//! via vault poll or explicitly request via RequestMek gossip.
//!
//! On-demand flow: member broadcasts RequestMek → operator receives → wraps for
//! the requester → sends MekTransfer directly to the requester. Point-to-point,
//! not mesh broadcast.
//!
//! Forward secrecy: after ban, ALL channel MEKs are rotated so the banned member
//! cannot decrypt future messages even if they retained the old MEK. This is
//! enforced by ban_member calling rekey_all_channels.

use rekindle_types::dht_types::{
    EncryptedMekCopy, MekVaultEntry, MemberSummary,
};
use rekindle_types::gossip_payload::{GossipPayload, ControlPayload};

use aws_lc_rs::rand::SecureRandom;
use crate::ChatError;
use super::super::CommunityService;

impl CommunityService {
    pub async fn rotate_mek(&self, gov_key: &str, channel_id: &str) -> Result<u64, ChatError> {
        let membership = self.require_operator(gov_key)?;
        let members = self.read_members(&membership.registry_key).await?;
        let generation = self.rekey_channel(gov_key, &membership.registry_key, channel_id, &members).await?;
        Ok(generation)
    }

    pub(crate) async fn rekey_channel(
        &self, gov_key: &str, registry_key: &str, channel_id: &str, members: &[MemberSummary],
    ) -> Result<u64, ChatError> {
        let reg_keypair = self.require_registry_keypair(registry_key)?;

        let current_gen = self.mek_cache.current(gov_key, channel_id).map_or(0, |(_, g)| g);
        let new_gen = current_gen + 1;

        let mut new_key = [0u8; 32];
        aws_lc_rs::rand::SystemRandom::new()
            .fill(&mut new_key)
            .map_err(|e| ChatError::Internal(format!("MEK gen: {e}")))?;

        let operator_seed = self.io.pseudonym_seed(gov_key)?;
        let operator_x25519_seed = blake3::derive_key("rekindle identity x25519 v1", &operator_seed);
        let mek_wire = crate::crypto::mek::mek_to_wire(&new_key, new_gen);

        let our_pseudonym_hex = self.io.pseudonym_hex(gov_key)?;

        let mut copies = Vec::with_capacity(members.len());
        for m in members {
            let Some(x25519_pub) = m.x25519_pub.as_ref()
                .and_then(|h| hex::decode(h).ok())
                .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()) else {
                tracing::warn!(
                    member = &m.pseudonym_key[..12.min(m.pseudonym_key.len())],
                    channel = channel_id,
                    "member has no x25519_pub — cannot wrap MEK. \
                     Member must rejoin to receive rotated MEKs."
                );
                continue;
            };

            match crate::crypto::mek::wrap_mek(&operator_x25519_seed, &x25519_pub, &mek_wire) {
                Ok(wrapped) => {
                    copies.push(EncryptedMekCopy {
                        target_pseudonym: m.pseudonym_key.clone(),
                        encrypted_mek: wrapped,
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        member = &m.pseudonym_key[..12.min(m.pseudonym_key.len())],
                        channel = channel_id,
                        error = %e,
                        "MEK wrap FAILED for member — they will not receive this MEK rotation"
                    );
                }
            }
        }

        let vault_entry = MekVaultEntry {
            channel_id: channel_id.to_string(),
            generation: new_gen,
            rotator_pseudonym: our_pseudonym_hex.clone(),
            copies,
        };

        let mut vault = self.read_mek_vault(registry_key).await.unwrap_or_default();
        if let Some(existing) = vault.iter_mut().find(|e| e.channel_id == channel_id) {
            *existing = vault_entry;
        } else {
            vault.push(vault_entry);
        }
        self.write_mek_vault(registry_key, &vault, &reg_keypair).await?;

        self.mek_cache.insert(gov_key, channel_id, new_key, new_gen);

        if let Err(e) = self.io.broadcast_gossip_dedup(
            gov_key,
            GossipPayload::Control(ControlPayload::MekRotated {
                channel_id: Some(channel_id.into()),
                new_generation: new_gen,
                rotator_pseudonym: Some(our_pseudonym_hex),
            }),
        ).await {
            tracing::debug!(
                channel = channel_id,
                generation = new_gen,
                error = %e,
                "MekRotated gossip failed — peers will discover via vault poll"
            );
        }

        tracing::info!(channel = channel_id, generation = new_gen, "MEK rotated + gossip broadcast");
        Ok(new_gen)
    }

    pub(crate) async fn rekey_all_channels(
        &self, gov_key: &str, registry_key: &str, members: &[MemberSummary],
    ) {
        let channels = self.read_channels(gov_key).await.unwrap_or_default();
        for ch in &channels {
            if let Err(e) = self.rekey_channel(gov_key, registry_key, &ch.id, members).await {
                tracing::warn!(channel = %ch.id, error = %e, "rekey failed");
            }
        }
    }

    pub async fn request_mek_from_operator(
        &self, gov_key: &str, channel_id: &str, needed_generation: u64,
    ) -> Result<(), ChatError> {
        let pseudonym_hex = self.io.pseudonym_hex(gov_key)?;

        if let Err(e) = self.io.broadcast_gossip_dedup(
            gov_key,
            GossipPayload::Control(ControlPayload::RequestMek {
                channel_id: channel_id.into(),
                needed_generation,
                requester_pseudonym: pseudonym_hex,
            }),
        ).await {
            tracing::warn!(
                governance = &gov_key[..12.min(gov_key.len())],
                channel = channel_id,
                generation = needed_generation,
                error = %e,
                "MEK request gossip failed — operator may be offline"
            );
            return Err(ChatError::Internal(format!("MEK request broadcast failed: {e}")));
        }

        tracing::info!(
            governance = &gov_key[..12.min(gov_key.len())],
            channel = channel_id,
            generation = needed_generation,
            "MEK request broadcast — awaiting operator response"
        );
        Ok(())
    }

    pub async fn handle_mek_request(
        &self, gov_key: &str, channel_id: &str, requester_pseudonym: &str, needed_generation: u64,
    ) -> Result<(), ChatError> {
        let membership = self.require_operator(gov_key)?;

        let Some(mek_key) = self.mek_cache.get_generation(gov_key, channel_id, needed_generation) else {
            tracing::warn!(governance = &gov_key[..12.min(gov_key.len())], channel = channel_id, generation = needed_generation, "MEK not cached — cannot fulfill request");
            return Err(ChatError::MekNotCached { community: gov_key.into(), channel: channel_id.into() });
        };

        let members = self.read_members(&membership.registry_key).await?;
        let requester_member = members.iter()
            .find(|m| m.pseudonym_key == requester_pseudonym)
            .ok_or_else(|| ChatError::Internal(format!("requester {} not in registry", &requester_pseudonym[..12.min(requester_pseudonym.len())])))?;

        let requester_x25519_pub: [u8; 32] = requester_member.x25519_pub.as_ref()
            .and_then(|h| hex::decode(h).ok())
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .ok_or_else(|| ChatError::Internal("requester has no x25519_pub".into()))?;

        let pseudonym_seed = self.io.pseudonym_seed(gov_key)?;
        let our_x25519_seed = blake3::derive_key("rekindle identity x25519 v1", &pseudonym_seed);
        let our_pseudonym_hex = self.io.pseudonym_hex(gov_key)?;

        let mek_wire = crate::crypto::mek::mek_to_wire(&mek_key, needed_generation);
        let wrapped = crate::crypto::mek::wrap_mek(&our_x25519_seed, &requester_x25519_pub, &mek_wire)?;

        self.io.send_gossip_direct(gov_key, requester_pseudonym,
            GossipPayload::Control(ControlPayload::MekTransfer {
                community_id: gov_key.into(),
                channel_id: Some(channel_id.into()),
                generation: needed_generation,
                sender_pseudonym: our_pseudonym_hex,
                wrapped_mek: wrapped,
            }),
        ).await.map_err(|e| ChatError::Internal(format!("MEK transfer send failed: {e}")))?;

        tracing::info!(governance = &gov_key[..12.min(gov_key.len())], channel = channel_id, generation = needed_generation, "MEK transferred");
        Ok(())
    }

    pub fn receive_mek_transfer(
        &self, gov_key: &str, channel_id: &str, generation: u64, sender_pseudonym_hex: &str, wrapped_mek: &[u8],
    ) -> Result<(), ChatError> {
        let pseudonym_seed = self.io.pseudonym_seed(gov_key)?;
        let our_x25519_seed = blake3::derive_key("rekindle identity x25519 v1", &pseudonym_seed);

        let sender_pub: [u8; 32] = hex::decode(sender_pseudonym_hex)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .ok_or_else(|| ChatError::Internal("invalid sender pseudonym hex".into()))?;

        let mek_wire = crate::crypto::mek::unwrap_mek(&our_x25519_seed, &sender_pub, wrapped_mek)
            .map_err(|e| {
                tracing::error!(governance = &gov_key[..12.min(gov_key.len())], channel = channel_id, generation, error = %e, "MEK unwrap FAILED");
                e
            })?;

        let (key, gen) = crate::crypto::mek::mek_from_wire(&mek_wire)?;
        self.mek_cache.insert(gov_key, channel_id, key, gen);

        tracing::info!(governance = &gov_key[..12.min(gov_key.len())], channel = channel_id, generation = gen, "MEK received and cached");
        Ok(())
    }
}
