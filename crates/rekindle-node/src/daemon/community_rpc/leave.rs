//! Leave handler (still RPC-based, best-effort).

use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_transport::payload::rpc::{CallResponse, CommunityLeaveNotification};

use super::{
    get_signing_key, get_transport, open_registry_writable, require_operator_registry,
    HANDLER_DEADLINE,
};

// ── Leave handler (still RPC-based, best-effort) ──────────────────────

/// Leave handler — remove member, rekey for forward secrecy.
pub(crate) async fn handle_leave(
    notif: &CommunityLeaveNotification,
    session: &RwLock<Option<rekindle_transport::Session>>,
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &RwLock<rekindle_transport::crypto::mek::MekCache>,
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
    session_path: &std::path::Path,
) -> CallResponse {
    if let Ok(response) = tokio::time::timeout(
        HANDLER_DEADLINE,
        handle_leave_inner(
            notif,
            session,
            signing_key,
            mek_cache,
            transport,
            session_path,
        ),
    )
    .await
    {
        response
    } else {
        tracing::error!(
            member = %&notif.leaving_pseudonym_hex[..16.min(notif.leaving_pseudonym_hex.len())],
            "leave handler exceeded deadline"
        );
        CallResponse::Ack
    }
}

async fn handle_leave_inner(
    notif: &CommunityLeaveNotification,
    session: &RwLock<Option<rekindle_transport::Session>>,
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &RwLock<rekindle_transport::crypto::mek::MekCache>,
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
    session_path: &std::path::Path,
) -> CallResponse {
    tracing::info!(
        community = %&notif.governance_key[..16.min(notif.governance_key.len())],
        member = %&notif.leaving_pseudonym_hex[..16.min(notif.leaving_pseudonym_hex.len())],
        "processing leave"
    );

    let Some(registry_key) = require_operator_registry(session, &notif.governance_key) else {
        return CallResponse::Ack;
    };
    let Some(transport_node) = get_transport(transport) else {
        return CallResponse::Ack;
    };
    let Ok(dht) = transport_node.dht() else {
        return CallResponse::Ack;
    };

    open_registry_writable(&transport_node, &registry_key).await;

    // Remove from index
    let mut members = dht
        .registry()
        .read_member_index(&registry_key)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "DHT read failed, using empty");
            Vec::new()
        });
    let before = members.len();
    members.retain(|m| m.pseudonym_key != notif.leaving_pseudonym_hex);
    if members.len() < before {
        let _ = dht
            .registry()
            .write_member_index(&registry_key, &members)
            .await;
        tracing::info!(remaining = members.len(), "member removed");
    }

    // Remove vault copies
    let mut vault = dht
        .registry()
        .read_mek_vault(&registry_key)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "DHT read failed, using empty");
            Vec::new()
        });
    for entry in &mut vault {
        entry
            .copies
            .retain(|c| c.target_pseudonym != notif.leaving_pseudonym_hex);
    }
    let _ = dht.registry().write_mek_vault(&registry_key, &vault).await;

    // Rekey for forward secrecy
    let Some(signing_key_bytes) = get_signing_key(signing_key) else {
        tracing::warn!("locked — cannot rekey");
        return CallResponse::Ack;
    };
    let channels = dht
        .governance()
        .read_channels(&notif.governance_key)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "DHT read failed, using empty");
            Vec::new()
        });
    let our_ps = rekindle_transport::crypto::pseudonym::derive_community_pseudonym(
        &signing_key_bytes,
        &notif.governance_key,
    );
    let our_ps_hex = hex::encode(our_ps.verifying_key().to_bytes());

    let mut new_vault = Vec::new();
    for channel in &channels {
        let gen = mek_cache
            .read()
            .current(&notif.governance_key, &channel.id)
            .map_or(0, rekindle_transport::crypto::mek::Mek::generation)
            + 1;
        let new_mek = rekindle_transport::crypto::mek::Mek::generate(gen);
        let mek_wire = new_mek.to_wire_bytes();
        let copies = crate::daemon::mek_wrap::wrap_for_members(&our_ps, &members, &mek_wire);
        new_vault.push(rekindle_transport::payload::dht_types::MekVaultEntry {
            channel_id: channel.id.clone(),
            generation: gen,
            rotator_pseudonym: our_ps_hex.clone(),
            copies,
        });
        mek_cache
            .write()
            .insert(&notif.governance_key, &channel.id, new_mek);
    }
    if !new_vault.is_empty() {
        let _ = dht
            .registry()
            .write_mek_vault(&registry_key, &new_vault)
            .await;
    }

    {
        let guard = session.read();
        if let Some(ref s) = *guard {
            let _ = s.save(session_path);
        }
    }
    tracing::info!(
        rekeyed = new_vault.len(),
        remaining = members.len(),
        "leave + rekey complete"
    );
    CallResponse::Ack
}
