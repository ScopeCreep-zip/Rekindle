//! Join-request moderation: approve, reject, pending list.

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;

use crate::daemon::dispatch::{state_error, DaemonContext};

pub(crate) async fn handle_approve(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
    member_pseudonym: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(governance_key) {
        Ok(m) => m,
        Err(e) => return e,
    };
    if !membership.is_operator {
        return IpcResponse::error(403, "not an operator for this community");
    }

    let signing_key = match ctx.require_signing_key() {
        Ok(k) => k,
        Err(e) => return e,
    };
    let dht = match transport.dht() {
        Ok(d) => d,
        Err(e) => return IpcResponse::error(500, format!("DHT: {e}")),
    };

    // Read moderation queue
    let mut queue = dht
        .registry()
        .read_moderation_queue(&membership.registry_key)
        .await
        .unwrap_or_default();
    let Some(pending) = queue
        .iter()
        .find(|p| p.requester_pseudonym_hex == member_pseudonym)
        .cloned()
    else {
        return IpcResponse::error(404, format!("no pending request from {member_pseudonym}"));
    };

    // Register member
    let mut members = dht
        .registry()
        .read_member_index(&membership.registry_key)
        .await
        .unwrap_or_default();
    let slot = members
        .iter()
        .map(|m| m.subkey_index)
        .max()
        .map_or(1, |m| m + 1)
        .max(1);
    members.push(rekindle_transport::payload::dht_types::MemberSummary {
        pseudonym_key: member_pseudonym.to_string(),
        display_name: pending.display_name,
        role_ids: Vec::new(),
        joined_at: rekindle_transport::timestamp_ms(),
        subkey_index: slot,
        onboarding_complete: true,
        timeout_until: None,
        profile_dht_key: Some(pending.profile_dht_key),
        channel_records: std::collections::HashMap::new(),
    });
    if let Err(e) = dht
        .registry()
        .write_member_index(&membership.registry_key, &members)
        .await
    {
        return IpcResponse::error(500, format!("member registration failed: {e}"));
    }

    // Remove from queue
    queue.retain(|p| p.requester_pseudonym_hex != member_pseudonym);
    let _ = dht
        .registry()
        .write_moderation_queue(&membership.registry_key, &queue)
        .await;

    // Wrap MEKs for the approved member
    let channels = dht
        .governance()
        .read_channels(&membership.governance_key)
        .await
        .unwrap_or_default();
    if let Ok(transfers) = rekindle_transport::operations::mek::wrap_meks_for_member(
        &channels,
        member_pseudonym,
        &signing_key,
        &membership.governance_key,
        &ctx.mek_cache,
    ) {
        let mut vault = dht
            .registry()
            .read_mek_vault(&membership.registry_key)
            .await
            .unwrap_or_default();
        for t in &transfers {
            if let Some(e) = vault.iter_mut().find(|e| e.channel_id == t.channel_id) {
                e.copies
                    .push(rekindle_transport::payload::dht_types::EncryptedMekCopy {
                        target_pseudonym: member_pseudonym.to_string(),
                        encrypted_mek: t.wrapped_mek.clone(),
                    });
            }
        }
        let _ = dht
            .registry()
            .write_mek_vault(&membership.registry_key, &vault)
            .await;
    }

    IpcResponse::ok(&serde_json::json!({ "approved": member_pseudonym, "slot": slot }))
}

pub(crate) async fn handle_reject(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
    member_pseudonym: &str,
    reason: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(governance_key) {
        Ok(m) => m,
        Err(e) => return e,
    };
    if !membership.is_operator {
        return IpcResponse::error(403, "not an operator for this community");
    }

    let dht = match transport.dht() {
        Ok(d) => d,
        Err(e) => return IpcResponse::error(500, format!("DHT: {e}")),
    };

    let mut queue = dht
        .registry()
        .read_moderation_queue(&membership.registry_key)
        .await
        .unwrap_or_default();
    queue.retain(|p| p.requester_pseudonym_hex != member_pseudonym);
    let _ = dht
        .registry()
        .write_moderation_queue(&membership.registry_key, &queue)
        .await;

    IpcResponse::ok(&serde_json::json!({ "rejected": member_pseudonym, "reason": reason }))
}

pub(crate) async fn handle_pending_members(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(governance_key) {
        Ok(m) => m,
        Err(e) => return e,
    };

    let dht = match transport.dht() {
        Ok(d) => d,
        Err(e) => return IpcResponse::error(500, format!("DHT: {e}")),
    };

    let queue = dht
        .registry()
        .read_moderation_queue(&membership.registry_key)
        .await
        .unwrap_or_default();
    IpcResponse::ok(&queue)
}
