//! MEK lifecycle operations — rotate, request, replenish prekeys.

use std::sync::Arc;

use parking_lot::RwLock;
use tracing::info;

use crate::crypto::mek::{Mek, MekCache};
use crate::error::{TransportError, Result};
use crate::node::TransportNode;
use crate::payload::dht_types::MekVaultEntry;
use crate::session::CommunityMembership;

/// Result of a MEK rotation.
pub struct MekRotated {
    /// The new MEK generation number.
    pub generation: u64,
    /// Number of vault copies written (one per member).
    pub copies_written: usize,
}

/// Rotate the MEK for a channel.
///
/// Steps:
/// 1. Generate a new MEK at the next generation
/// 2. Read the member registry to get all member pseudonym keys
/// 3. Wrap the MEK for each member via ECDH
/// 4. Write wrapped copies to the MEK vault in the registry
/// 5. Cache the new MEK locally
/// 6. Return the generation and copy count (caller broadcasts MekRotated gossip)
pub async fn rotate_mek(
    node: &TransportNode,
    membership: &CommunityMembership,
    channel_id: &str,
    mek_cache: &Arc<RwLock<MekCache>>,
    signing_key_bytes: &[u8; 32],
) -> Result<MekRotated> {
    info!(
        channel = channel_id,
        community = %membership.community_name,
        "rotating MEK"
    );

    // Determine next generation
    let current_gen = mek_cache
        .read()
        .current(&membership.governance_key, channel_id)
        .map_or(0, Mek::generation);
    let new_gen = current_gen + 1;

    // Generate new MEK
    let new_mek = Mek::generate(new_gen);
    let mek_wire = new_mek.to_wire_bytes();

    // Read member registry to get all pseudonym keys
    let dht = node.dht()?;
    let members = dht
        .registry()
        .read_member_index(&membership.registry_key)
        .await?;

    // Wrap MEK for each member
    let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);
    let mut copies = Vec::with_capacity(members.len());

    for member in &members {
        let pub_bytes = match hex::decode(&member.pseudonym_key) {
            Ok(b) if b.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&b);
                arr
            }
            _ => {
                tracing::warn!(
                    pseudonym = %member.pseudonym_key,
                    "skipping member with invalid pseudonym key"
                );
                continue;
            }
        };

        match crate::crypto::mek::wrap_mek(&signing_key, &pub_bytes, &mek_wire) {
            Ok(wrapped) => {
                copies.push(crate::payload::dht_types::EncryptedMekCopy {
                    target_pseudonym: member.pseudonym_key.clone(),
                    encrypted_mek: wrapped,
                });
            }
            Err(e) => {
                tracing::warn!(
                    pseudonym = %member.pseudonym_key,
                    error = %e,
                    "MEK wrap failed for member, skipping"
                );
            }
        }
    }

    let copies_written = copies.len();

    // Write to MEK vault
    let vault_entry = MekVaultEntry {
        channel_id: channel_id.to_string(),
        generation: new_gen,
        rotator_pseudonym: membership.pseudonym_key.clone(),
        copies,
    };

    // Read existing vault, append this entry, write back
    let mut vault = dht
        .registry()
        .read_mek_vault(&membership.registry_key)
        .await
        .unwrap_or_default();

    // Replace existing entry for this channel or append
    if let Some(existing) = vault.iter_mut().find(|e| e.channel_id == channel_id) {
        *existing = vault_entry;
    } else {
        vault.push(vault_entry);
    }

    dht.registry()
        .write_mek_vault(&membership.registry_key, &vault)
        .await?;

    // Cache locally
    mek_cache
        .write()
        .insert(&membership.governance_key, channel_id, new_mek);

    info!(
        channel = channel_id,
        generation = new_gen,
        copies = copies_written,
        "MEK rotated"
    );

    Ok(MekRotated {
        generation: new_gen,
        copies_written,
    })
}

/// Build a serialized `RequestMek` gossip payload for the CLI to broadcast.
///
/// The CLI signs and broadcasts this to the community mesh. Peers with
/// the MEK respond with a `MekTransfer` gossip message.
pub fn build_mek_request_payload(
    channel_id: &str,
    needed_generation: u64,
    our_pseudonym: &str,
) -> Result<Vec<u8>> {
    let payload = crate::payload::gossip::GossipPayload::Control(
        crate::payload::gossip::ControlPayload::RequestMek {
            channel_id: channel_id.to_string(),
            needed_generation,
            requester_pseudonym: our_pseudonym.to_string(),
        },
    );
    postcard::to_stdvec(&payload)
        .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })
}

/// Replenish prekeys by generating a new bundle and publishing to profile DHT.
pub async fn replenish_prekeys(
    node: &TransportNode,
    profile_dht_key: &str,
    signing_key_bytes: &[u8; 32],
) -> Result<u32> {
    let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);
    let x25519_secret = crate::crypto::pseudonym::pseudonym_to_x25519(&signing_key);
    let x25519_public = x25519_dalek::PublicKey::from(&x25519_secret);

    let signal = crate::crypto::signal_session::SignalSessionManager::new(
        Box::new(crate::crypto::signal_store::MemoryIdentityStore::new(
            x25519_secret.to_bytes().to_vec(),
            x25519_public.as_bytes().to_vec(),
            1,
        )),
        Box::new(crate::crypto::signal_store::MemoryPreKeyStore::new()),
        Box::new(crate::crypto::signal_store::MemorySessionStore::new()),
    );

    let bundle = signal
        .generate_prekey_bundle(1, Some(1))
        .map_err(|e| TransportError::IdentityCreationFailed {
            step: "prekey replenish".into(),
            reason: e.to_string(),
        })?;

    let bundle_bytes = bundle.to_bytes().map_err(|e| TransportError::SerializationFailed {
        reason: format!("prekey bundle: {e}"),
    })?;

    let byte_count = bundle_bytes.len();

    let dht = node.dht()?;
    dht.profile()
        .set_subkey(
            profile_dht_key,
            crate::payload::dht_types::PROFILE_SUBKEY_PREKEY_BUNDLE,
            bundle_bytes,
        )
        .await?;

    #[allow(clippy::cast_possible_truncation)]
    let count = byte_count as u32;
    info!(bytes = byte_count, "prekeys replenished");
    Ok(count)
}
