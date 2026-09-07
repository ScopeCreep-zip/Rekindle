//! Rekey implementation — MEK generation and distribution after
//! ban/leave/rotate events.

use parking_lot::RwLock;

use crate::daemon::community_rpc::get_signing_key;

// `rekey_all_channels` lived here: it rekeyed every channel after a
// ban, distributing new MEKs through the registry's MEK vault. It was
// reachable only from the coordinator Ban RPC, which v2.0 removed — the
// daemon's own IPC ban never called it — so deleting it costs no
// reachable behaviour, and the vault it wrote is itself being retired.
//
// The v2.0 replacement is `rekindle_mek_rotation::rotate_text_mek_for_departure`,
// which distributes peer-to-peer. The daemon needs a `rekindle-mek-rotation`
// Deps adapter to call it; until then the governance adapter's
// `spawn_text_mek_rotation_for_ban` traces the gap rather than pretending
// to rotate.

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
        let copies: Vec<rekindle_transport::payload::dht_types::EncryptedMekCopy> =
            crate::daemon::mek_wrap::wrap_for_members(&ps, members, &mek_wire);

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
