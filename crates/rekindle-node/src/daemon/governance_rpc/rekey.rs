//! Rekey implementation — MEK generation and distribution after
//! ban/leave/rotate events.

use parking_lot::RwLock;

use crate::daemon::community_rpc::get_signing_key;

/// Rekey all channels in a community. Used after ban/leave for forward secrecy.
pub(super) async fn rekey_all_channels(
    dht: &rekindle_transport::DhtStore,
    gov_key: &str,
    registry_key: &str,
    members: &[rekindle_transport::payload::dht_types::MemberSummary],
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &RwLock<rekindle_transport::crypto::mek::MekCache>,
) {
    let channels = dht
        .governance()
        .read_channels(gov_key)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "DHT read failed, using empty");
            Vec::new()
        });
    let channel_ids: Vec<String> = channels.iter().map(|ch| ch.id.clone()).collect();
    rekey_channels(
        dht,
        gov_key,
        registry_key,
        &channel_ids,
        members,
        signing_key,
        mek_cache,
    )
    .await;
}

/// Rekey specific channels: generate new MEKs, wrap for all members, write vault.
pub(super) async fn rekey_channels(
    dht: &rekindle_transport::DhtStore,
    gov_key: &str,
    registry_key: &str,
    channel_ids: &[String],
    members: &[rekindle_transport::payload::dht_types::MemberSummary],
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &RwLock<rekindle_transport::crypto::mek::MekCache>,
) {
    let Some(sk) = get_signing_key(signing_key) else {
        tracing::warn!("daemon locked — cannot rekey");
        return;
    };
    let ps = rekindle_transport::crypto::pseudonym::derive_community_pseudonym(&sk, gov_key);
    let ps_hex = hex::encode(ps.verifying_key().to_bytes());

    let mut new_vault_entries = Vec::new();

    for channel_id in channel_ids {
        let current_gen = mek_cache
            .read()
            .current(gov_key, channel_id)
            .map_or(0, rekindle_transport::crypto::mek::Mek::generation);
        let new_gen = current_gen + 1;
        let new_mek = rekindle_transport::crypto::mek::Mek::generate(new_gen);
        let mek_wire = new_mek.to_wire_bytes();

        // Wrap for each remaining member
        let copies: Vec<rekindle_transport::payload::dht_types::EncryptedMekCopy> = members
            .iter()
            .filter_map(|m| {
                let pub_bytes: [u8; 32] = hex::decode(&m.pseudonym_key).ok()?.try_into().ok()?;
                rekindle_transport::crypto::mek::wrap_mek(&ps, &pub_bytes, &mek_wire)
                    .ok()
                    .map(
                        |wrapped| rekindle_transport::payload::dht_types::EncryptedMekCopy {
                            target_pseudonym: m.pseudonym_key.clone(),
                            encrypted_mek: wrapped,
                        },
                    )
            })
            .collect();

        new_vault_entries.push(rekindle_transport::payload::dht_types::MekVaultEntry {
            channel_id: channel_id.clone(),
            generation: new_gen,
            rotator_pseudonym: ps_hex.clone(),
            copies,
        });

        // Cache locally
        mek_cache.write().insert(gov_key, channel_id, new_mek);
        tracing::info!(channel = %channel_id, generation = new_gen, copies = new_vault_entries.last().map_or(0, |e| e.copies.len()), "channel rekeyed");
    }

    if !new_vault_entries.is_empty() {
        let _ = dht
            .registry()
            .write_mek_vault(registry_key, &new_vault_entries)
            .await;
    }
}
