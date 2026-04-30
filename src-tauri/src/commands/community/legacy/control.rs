use crate::state::SharedState;
use crate::state_helpers;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

/// Perform local MEK rotation: generate, wrap per-member, write vault, broadcast.
pub(crate) async fn rotate_mek_local(
    state: &SharedState,
    community_id: &str,
    keystore: &crate::keystore::KeystoreHandle,
) -> Result<(), String> {
    use rekindle_crypto::group::media_key::MediaEncryptionKey;
    use rekindle_crypto::group::mek_distribution::wrap_mek;
    use rekindle_protocol::dht::community::member_registry;
    use rekindle_protocol::dht::community::types::{EncryptedMEKCopy, MEKVaultEntry};

    let current_gen = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .map_or(0, |c| c.mek_generation)
    };
    let new_gen = current_gen + 1;
    let mek = MediaEncryptionKey::generate(new_gen);

    let (my_signing_key, my_pseudonym, registry_key, registry_owner_kp) = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        let registry_key = c
            .member_registry_key
            .clone()
            .ok_or("no member registry key")?;
        let registry_kp = c
            .registry_owner_keypair
            .clone()
            .ok_or("no registry_owner_keypair — only admins can rotate MEK")?;
        let my_pseudonym = c.my_pseudonym_key.clone().ok_or("no pseudonym key")?;
        let secret = state.identity_secret.lock();
        let signing_key = match *secret {
            Some(ref s) => {
                rekindle_crypto::group::pseudonym::derive_community_pseudonym(s, community_id)
            }
            None => return Err("no identity secret".into()),
        };
        (signing_key, my_pseudonym, registry_key, registry_kp)
    };

    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    if let Ok(kp) = registry_owner_kp.parse::<veilid_core::KeyPair>() {
        if let Err(e) = mgr.open_record_writable(&registry_key, kp).await {
            tracing::warn!(error = %e, "failed to open registry writable for MEK rotation");
        }
    }

    let members = member_registry::read_member_index(&mgr, &registry_key)
        .await
        .map_err(|e| format!("read member index: {e}"))?;

    let mek_wire = mek.to_wire_bytes();
    let mut copies = Vec::with_capacity(members.len());
    for member in &members {
        let Some(pub_bytes): Option<[u8; 32]> = hex::decode(&member.pseudonym_key)
            .ok()
            .and_then(|b| b.try_into().ok())
        else {
            tracing::warn!(
                member = %member.pseudonym_key,
                "skipping MEK wrap — invalid pseudonym key"
            );
            continue;
        };
        match wrap_mek(&my_signing_key, &pub_bytes, &mek_wire) {
            Ok(encrypted) => {
                copies.push(EncryptedMEKCopy {
                    target_pseudonym: member.pseudonym_key.clone(),
                    encrypted_mek: encrypted,
                });
            }
            Err(e) => {
                tracing::warn!(
                    member = %member.pseudonym_key,
                    error = %e,
                    "failed to wrap MEK for member"
                );
            }
        }
    }

    let vault_entry = MEKVaultEntry {
        channel_id: String::new(),
        generation: new_gen,
        rotator_pseudonym: my_pseudonym.clone(),
        copies,
    };
    member_registry::write_mek_vault(&mgr, &registry_key, &[vault_entry])
        .await
        .map_err(|e| format!("write MEK vault: {e}"))?;

    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.mek_generation = new_gen;
        }
    }
    state.mek_cache.lock().insert(community_id.to_string(), mek);

    if let Some(ref ks) = *keystore.lock() {
        if let Some(mek) = state.mek_cache.lock().get(community_id) {
            crate::keystore::persist_mek(ks, community_id, mek);
        }
    }

    let envelope = CommunityEnvelope::Control(ControlPayload::MEKRotated {
        channel_id: None,
        new_generation: new_gen,
        rotator_pseudonym: None,
    });
    let _ = crate::services::community::send_to_mesh(state, community_id, &envelope);

    Ok(())
}
