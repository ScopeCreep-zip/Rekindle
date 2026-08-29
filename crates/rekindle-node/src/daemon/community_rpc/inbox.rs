//! Process pending join requests from the inbox DHT record.

use std::sync::Arc;

use parking_lot::RwLock;

use super::inbox_stages::{notify_new_members, process_inbox_joins, process_inbox_leaves};
use super::{get_signing_key, get_transport, open_registry_writable, require_operator_registry};

pub async fn process_inbox(
    session: &RwLock<Option<rekindle_transport::Session>>,
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &RwLock<rekindle_transport::crypto::mek::MekCache>,
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
    session_path: &std::path::Path,
    governance_key: &str,
) {
    let Some(registry_key) = require_operator_registry(session, governance_key) else {
        tracing::trace!(governance_key, "inbox: not operator for this community");
        return;
    };
    let Some(signing_key_bytes) = get_signing_key(signing_key) else {
        tracing::warn!(
            governance_key,
            "inbox: signing key not available (daemon locked?)"
        );
        return;
    };
    let Some(transport_node) = get_transport(transport) else {
        tracing::warn!(governance_key, "inbox: transport not started");
        return;
    };
    let Ok(dht) = transport_node.dht() else {
        tracing::warn!(governance_key, "inbox: DHT access failed");
        return;
    };

    // Read metadata for inbox key
    if let Err(e) =
        rekindle_transport::broadcast::dht_writes::open_readonly(&transport_node, governance_key)
            .await
    {
        tracing::warn!(governance_key, error = %e, "inbox: cannot open governance record");
        return;
    }
    let metadata = match dht.governance().read_metadata(governance_key).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            tracing::warn!(governance_key, "inbox: governance metadata is None");
            return;
        }
        Err(e) => {
            tracing::warn!(governance_key, error = %e, "inbox: governance metadata read failed");
            return;
        }
    };
    // If join_inbox_key is empty, the local cache may be stale (initial metadata
    // from step 1 of create, before step 9 wrote the final metadata). Retry with
    // force_refresh=true to pull the latest from the DHT network.
    let metadata = if metadata.join_inbox_key.is_empty() {
        if let Ok(Some(data)) = rekindle_transport::broadcast::dht_writes::get(
            &transport_node,
            governance_key,
            rekindle_transport::payload::dht_types::MANIFEST_METADATA,
            true,
        )
        .await
        {
            match serde_json::from_slice::<rekindle_transport::payload::dht_types::CommunityMetadata>(
                &data,
            ) {
                Ok(m) if !m.join_inbox_key.is_empty() => m,
                Ok(_) => {
                    tracing::warn!(
                        governance_key,
                        "inbox: join_inbox_key still empty after network refresh"
                    );
                    return;
                }
                Err(e) => {
                    tracing::warn!(governance_key, error = %e, "inbox: metadata parse failed after refresh");
                    return;
                }
            }
        } else {
            tracing::warn!(
                governance_key,
                "inbox: metadata fetch failed on network refresh"
            );
            return;
        }
    } else {
        metadata
    };

    // Open inbox readonly to read pending requests
    if let Err(e) = rekindle_transport::broadcast::dht_writes::open_readonly(
        &transport_node,
        &metadata.join_inbox_key,
    )
    .await
    {
        tracing::warn!(inbox_key = %metadata.join_inbox_key, error = %e, "inbox: cannot open inbox record");
        return;
    }
    let pending = match rekindle_transport::operations::community::read_inbox_requests(
        &dht,
        &metadata.join_inbox_key,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(inbox_key = %metadata.join_inbox_key, error = %e, "inbox: read_inbox_requests failed");
            return;
        }
    };

    if pending.is_empty() {
        tracing::trace!(governance_key, "inbox: no pending requests");
        return;
    }
    tracing::info!(community = %metadata.name, requests = pending.len(), "processing join inbox");

    // Open registry writable
    open_registry_writable(&transport_node, &registry_key).await;

    // Read current state
    let bans = dht
        .governance()
        .read_bans(governance_key)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "DHT read failed, using empty");
            Vec::new()
        });
    let mut members = dht
        .registry()
        .read_member_index(&registry_key)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "DHT read failed, using empty");
            Vec::new()
        });
    let channels = dht
        .governance()
        .read_channels(governance_key)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "DHT read failed, using empty");
            Vec::new()
        });

    // Process leave entries first — remove members and rekey
    process_inbox_leaves(
        &dht,
        &registry_key,
        governance_key,
        &mut members,
        &channels,
        signing_key,
        mek_cache,
        &pending,
    )
    .await;

    // Process join entries
    let new_members = process_inbox_joins(
        &dht,
        &registry_key,
        &metadata,
        &bans,
        &mut members,
        &pending,
    )
    .await;

    if new_members == 0 {
        return;
    }

    // Write updated member index
    if let Err(e) = dht
        .registry()
        .write_member_index(&registry_key, &members)
        .await
    {
        tracing::error!(error = %e, "inbox: failed to write member index");
        return;
    }

    // Wrap MEKs for all new members and update vault
    let mut vault = dht
        .registry()
        .read_mek_vault(&registry_key)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "DHT read failed, using empty");
            Vec::new()
        });
    for req in &pending {
        if bans
            .iter()
            .any(|b| b.pseudonym_key == req.requester_pseudonym_hex)
        {
            continue;
        }
        if matches!(
            metadata.join_policy,
            rekindle_transport::payload::dht_types::JoinPolicy::WaitingRoom
        ) {
            continue;
        }

        match rekindle_transport::operations::mek::wrap_meks_for_member(
            &channels,
            &req.requester_pseudonym_hex,
            &signing_key_bytes,
            governance_key,
            mek_cache,
        ) {
            Ok(transfers) => {
                for t in &transfers {
                    if let Some(entry) = vault.iter_mut().find(|e| e.channel_id == t.channel_id) {
                        entry.copies.push(
                            rekindle_transport::payload::dht_types::EncryptedMekCopy {
                                target_pseudonym: req.requester_pseudonym_hex.clone(),
                                encrypted_mek: t.wrapped_mek.clone(),
                            },
                        );
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "inbox: MEK wrap failed"),
        }
    }
    let _ = dht.registry().write_mek_vault(&registry_key, &vault).await;

    // Direct notification to newly approved members (tier 2 — instant).
    notify_new_members(
        &transport_node,
        governance_key,
        &registry_key,
        &members,
        &vault,
        &signing_key_bytes,
        &bans,
        &pending,
    )
    .await;

    // Save session
    {
        let guard = session.read();
        if let Some(ref s) = *guard {
            let _ = s.save(session_path);
        }
    }
    tracing::info!(community = %metadata.name, new_members, total = members.len(), "inbox processing complete");
}
