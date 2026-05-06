//! MEK lifecycle operations — rotate, request, wrap/unwrap, replenish prekeys.
//!
//! Typed reads/writes via `dht/registry.rs` and `dht/profile.rs`.
//! Raw DHT I/O via `broadcast::dht_writes` for profile subkey writes.

use std::sync::Arc;

use parking_lot::RwLock;
use tracing::info;

use crate::crypto::mek::{Mek, MekCache};
use crate::error::{TransportError, Result};
use crate::broadcast::node::TransportNode;
use crate::payload::dht_types::MekVaultEntry;
use crate::session::CommunityMembership;

#[derive(Debug, Clone, serde::Serialize)]
pub struct MekRotated {
    pub generation: u64,
    pub copies_written: usize,
}

pub async fn rotate_mek(
    node: &TransportNode, membership: &CommunityMembership, channel_id: &str,
    mek_cache: &Arc<RwLock<MekCache>>, signing_key_bytes: &[u8; 32],
) -> Result<MekRotated> {
    info!(channel = channel_id, community = %membership.community_name, "rotating MEK");
    let dht = node.dht()?;

    let current_gen = mek_cache.read().current(&membership.governance_key, channel_id).map_or(0, Mek::generation);
    let new_gen = current_gen + 1;
    let new_mek = Mek::generate(new_gen);
    let mek_wire = new_mek.to_wire_bytes();

    let members = dht.registry().read_member_index(&membership.registry_key).await?;
    let our_pseudonym = crate::crypto::pseudonym::derive_community_pseudonym(signing_key_bytes, &membership.governance_key);
    let mut copies = Vec::with_capacity(members.len());

    for member in &members {
        let pub_bytes = match hex::decode(&member.pseudonym_key) {
            Ok(b) if b.len() == 32 => { let mut arr = [0u8; 32]; arr.copy_from_slice(&b); arr }
            _ => { tracing::warn!(pseudonym = %member.pseudonym_key, "skipping invalid pseudonym"); continue; }
        };
        match crate::crypto::mek::wrap_mek(&our_pseudonym, &pub_bytes, &mek_wire) {
            Ok(wrapped) => copies.push(crate::payload::dht_types::EncryptedMekCopy {
                target_pseudonym: member.pseudonym_key.clone(), encrypted_mek: wrapped,
            }),
            Err(e) => { tracing::warn!(pseudonym = %member.pseudonym_key, error = %e, "MEK wrap failed"); }
        }
    }

    let copies_written = copies.len();
    let vault_entry = MekVaultEntry {
        channel_id: channel_id.to_string(), generation: new_gen,
        rotator_pseudonym: membership.pseudonym_key.clone(), copies,
    };

    let mut vault = dht.registry().read_mek_vault(&membership.registry_key).await.unwrap_or_default();
    if let Some(existing) = vault.iter_mut().find(|e| e.channel_id == channel_id) {
        *existing = vault_entry;
    } else { vault.push(vault_entry); }
    dht.registry().write_mek_vault(&membership.registry_key, &vault).await?;
    mek_cache.write().insert(&membership.governance_key, channel_id, new_mek);

    info!(channel = channel_id, generation = new_gen, copies = copies_written, "MEK rotated");
    Ok(MekRotated { generation: new_gen, copies_written })
}

pub fn build_mek_request_payload(channel_id: &str, needed_generation: u64, our_pseudonym: &str) -> Result<Vec<u8>> {
    postcard::to_stdvec(&crate::payload::gossip::GossipPayload::Control(
        crate::payload::gossip::ControlPayload::RequestMek {
            channel_id: channel_id.to_string(), needed_generation,
            requester_pseudonym: our_pseudonym.to_string(),
        },
    )).map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })
}

fn parse_pseudonym_pub(hex_str: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(hex_str).map_err(|e| TransportError::Internal(format!("invalid pseudonym hex: {e}")))?;
    let arr: [u8; 32] = bytes.try_into().map_err(|_| TransportError::Internal("pseudonym key wrong length".into()))?;
    Ok(arr)
}

pub fn receive_mek_transfer_payload(
    transfer: &crate::payload::rpc::MekTransferPayload,
    signing_key_bytes: &[u8; 32], governance_key: &str, mek_cache: &Arc<RwLock<MekCache>>,
) -> Result<u64> {
    let rotator_pub = parse_pseudonym_pub(&transfer.rotator_pseudonym_hex)?;
    let our_pseudonym = crate::crypto::pseudonym::derive_community_pseudonym(signing_key_bytes, governance_key);
    let mek_wire = crate::crypto::mek::unwrap_mek(&our_pseudonym, &rotator_pub, &transfer.wrapped_mek)?;
    let mek = Mek::from_wire_bytes(&mek_wire)
        .ok_or_else(|| TransportError::MekUnwrapFailed { reason: "invalid MEK wire bytes".into() })?;
    let gen = mek.generation();
    mek_cache.write().insert(governance_key, &transfer.channel_id, mek);
    info!(governance_key, channel_id = %transfer.channel_id, generation = gen, "MEK cached");
    Ok(gen)
}

pub fn wrap_meks_for_member(
    channels: &[crate::payload::dht_types::ChannelEntry],
    recipient_pseudonym_hex: &str, signing_key_bytes: &[u8; 32],
    governance_key: &str, mek_cache: &RwLock<MekCache>,
) -> Result<Vec<crate::payload::rpc::MekTransferPayload>> {
    let recipient_pub = parse_pseudonym_pub(recipient_pseudonym_hex)?;
    let our_pseudonym = crate::crypto::pseudonym::derive_community_pseudonym(signing_key_bytes, governance_key);
    let our_pseudonym_hex = hex::encode(our_pseudonym.verifying_key().to_bytes());
    let cache = mek_cache.read();
    let mut transfers = Vec::with_capacity(channels.len());
    for channel in channels {
        let Some(mek) = cache.current(governance_key, &channel.id) else {
            tracing::warn!(channel = %channel.name, "no MEK cached — skipping");
            continue;
        };
        let wrapped = crate::crypto::mek::wrap_mek(&our_pseudonym, &recipient_pub, &mek.to_wire_bytes())?;
        transfers.push(crate::payload::rpc::MekTransferPayload {
            channel_id: channel.id.clone(), generation: mek.generation(),
            rotator_pseudonym_hex: our_pseudonym_hex.clone(), wrapped_mek: wrapped,
        });
    }
    Ok(transfers)
}

/// Replenish prekeys — generate new bundle, publish to profile DHT via raw primitive.
pub async fn replenish_prekeys(
    node: &TransportNode, profile_dht_key: &str, signing_key_bytes: &[u8; 32],
) -> Result<u32> {
    let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);
    let x25519_secret = crate::crypto::pseudonym::pseudonym_to_x25519(&signing_key);
    let x25519_public = x25519_dalek::PublicKey::from(&x25519_secret);
    let signal = crate::crypto::signal_session::SignalSessionManager::new(
        Box::new(crate::crypto::signal_store::MemoryIdentityStore::new(
            x25519_secret.to_bytes().to_vec(), x25519_public.as_bytes().to_vec(), 1,
        )),
        Box::new(crate::crypto::signal_store::MemoryPreKeyStore::new()),
        Box::new(crate::crypto::signal_store::MemorySessionStore::new()),
    );
    let bundle = signal.generate_prekey_bundle(1, Some(1))
        .map_err(|e| TransportError::IdentityCreationFailed { step: "prekey replenish".into(), reason: e.to_string() })?;
    let bundle_bytes = bundle.to_bytes()
        .map_err(|e| TransportError::SerializationFailed { reason: format!("prekey bundle: {e}") })?;
    let byte_count = bundle_bytes.len();
    crate::broadcast::dht_writes::set(
        node, profile_dht_key, crate::payload::dht_types::PROFILE_SUBKEY_PREKEY_BUNDLE, bundle_bytes, None,
    ).await?;
    #[allow(clippy::cast_possible_truncation)]
    let count = byte_count as u32;
    info!(bytes = byte_count, "prekeys replenished");
    Ok(count)
}
