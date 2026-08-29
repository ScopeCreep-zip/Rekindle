//! Community lifecycle: create, join, leave.

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;
use crate::validation;

use super::ownership::write_encrypted_backup;
use crate::daemon::dispatch::{state_error, DaemonContext};

pub(crate) async fn handle_create(
    ctx: &DaemonContext,
    state: DaemonState,
    name: &str,
    description: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let name = match validation::validate_name(name, "Community") {
        Ok(n) => n,
        Err(e) => return e,
    };

    // Refuse if a community with this name already exists in the session
    {
        let guard = ctx.session.read();
        if let Some(ref s) = *guard {
            if s.community_by_name(&name).is_some() {
                return IpcResponse::error(409, format!(
                    "community '{name}' already exists — use a different name or leave the existing one first",
                ));
            }
        }
    }

    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let signing_key = match ctx.require_signing_key() {
        Ok(k) => k,
        Err(e) => return e,
    };
    let session = match ctx.require_session(Clone::clone) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let desc = if description.is_empty() {
        None
    } else {
        Some(description)
    };
    match rekindle_transport::operations::community::create_community(
        &transport,
        &session,
        &name,
        desc,
        &ctx.mek_cache,
        &signing_key,
    )
    .await
    {
        Ok(result) => {
            let gov_key_short = if result.governance_key.len() > 12 {
                &result.governance_key[..12]
            } else {
                &result.governance_key
            };
            let membership = rekindle_transport::session::CommunityMembership {
                governance_key: result.governance_key.clone(),
                pseudonym_key: result.our_pseudonym_key.clone(),
                display_name: session.identity.display_name.clone(),
                role_ids: Vec::new(),
                slot_index: 0,
                registry_key: result.registry_key.clone(),
                community_name: name.clone(),
                slot_seed: None,
                channel_record_keys: std::collections::HashMap::new(),
                community_mailbox_key: result.community_mailbox_key.clone(),
                join_inbox_key: result.join_inbox_key.clone(),
                is_operator: true,
                governance_keypair_label: Some(format!("community-governance-{gov_key_short}")),
            };
            {
                let mut guard = ctx.session.write();
                if let Some(ref mut s) = *guard {
                    s.join_community(membership);
                }
            }
            if let Err(e) = ctx.save_session() {
                return e;
            }

            // Store governance and registry keypairs. These MUST persist —
            // without them, the community cannot process joins or govern.
            if !result.governance_keypair_bytes.is_empty() {
                if let Err(e) = crate::state::keystore::store_governance_keypair(
                    gov_key_short,
                    &result.governance_keypair_bytes,
                )
                .await
                {
                    return IpcResponse::error(
                        500,
                        format!(
                            "community created but governance keypair storage failed: {e}. \
                         The community will not function. Delete and recreate."
                        ),
                    );
                }
            }
            if !result.registry_keypair_bytes.is_empty() {
                let reg_key_short = if result.registry_key.len() > 12 {
                    &result.registry_key[..12]
                } else {
                    &result.registry_key
                };
                if let Err(e) = crate::state::keystore::store_keypair_bytes(
                    &format!("registry-{reg_key_short}"),
                    &result.registry_keypair_bytes,
                )
                .await
                {
                    return IpcResponse::error(
                        500,
                        format!(
                            "community created but registry keypair storage failed: {e}. \
                         The community will not function. Delete and recreate."
                        ),
                    );
                }
            }

            // Best-effort encrypted backup of governance + registry keypairs.
            // Recovery path if the OS keyring is lost (migration, container rebuild).
            // Encrypted with the signing key so only the identity owner can recover.
            if let Some(ref sk_handle) = *ctx.signing_key.read() {
                let backup_dir = ctx
                    .session_path
                    .parent()
                    .unwrap_or(std::path::Path::new("."));
                let gov_backup = backup_dir.join(format!("governance-backup-{gov_key_short}.enc"));
                let reg_backup = backup_dir.join(format!(
                    "registry-backup-{}.enc",
                    if result.registry_key.len() > 12 {
                        &result.registry_key[..12]
                    } else {
                        &result.registry_key
                    }
                ));
                let key = sk_handle.as_bytes();
                if let Err(e) =
                    write_encrypted_backup(&gov_backup, &result.governance_keypair_bytes, key)
                {
                    tracing::warn!(error = %e, "governance keypair backup failed — keyring is the only copy");
                }
                if let Err(e) =
                    write_encrypted_backup(&reg_backup, &result.registry_keypair_bytes, key)
                {
                    tracing::warn!(error = %e, "registry keypair backup failed — keyring is the only copy");
                }
            }

            // Register gossip mesh for the new community (synchronous, no await needed).
            // The join inbox Veilid-level watch was already established during create_community
            // (transport layer). ValueChange events route through DaemonHandler::on_value_change
            // which checks session.communities for join_inbox_key matches — no SubscriptionManager
            // WatchRegistry registration needed.
            {
                let bcast_guard = ctx.broadcast_mgr.read();
                if let Some(ref bcast_mgr) = *bcast_guard {
                    bcast_mgr.register_mesh(&result.governance_key);
                    tracing::info!(community = %name, "gossip mesh registered for new community");
                }
            }

            IpcResponse::ok(&serde_json::json!({
                "governance_key": result.governance_key,
                "registry_key": result.registry_key,
                "community_mailbox_key": result.community_mailbox_key,
                "name": name,
            }))
        }
        Err(e) => IpcResponse::error(500, format!("community create failed: {e}")),
    }
}

pub(crate) async fn handle_join(
    ctx: &DaemonContext,
    state: DaemonState,
    invite: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = validation::validate_key(invite, "invite/governance key") {
        return e;
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let signing_key = match ctx.require_signing_key() {
        Ok(k) => k,
        Err(e) => return e,
    };
    let session = match ctx.require_session(Clone::clone) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // Phase 1: Submit join request to DHT inbox (non-blocking)
    tracing::info!(
        governance = invite,
        "handle_join: phase 1 — submitting join request"
    );
    let submitted = match rekindle_transport::operations::community::submit_join_request(
        &transport,
        &session,
        invite,
        &session.identity.display_name,
        &signing_key,
    )
    .await
    {
        Ok(s) => {
            tracing::info!(
                community = %s.community_name, governance = %s.governance_key,
                registry = %s.registry_key, pseudonym = %&s.our_pseudonym_hex[..16],
                "handle_join: phase 1 complete — request submitted"
            );
            s
        }
        Err(e) => return IpcResponse::error(500, format!("community join failed: {e}")),
    };

    // Register a pending join oneshot for tier 2 (direct notification from operator)
    let (notify_tx, notify_rx) = tokio::sync::oneshot::channel::<u32>();
    ctx.pending_joins.lock().insert(
        submitted.governance_key.clone(),
        (notify_tx, std::time::Instant::now()),
    );
    tracing::info!(community = %submitted.community_name, "handle_join: phase 2 — awaiting approval (tier 2 + tier 3)");

    // Phase 2: Await approval via tier 2 (direct) + tier 3 (poll)
    let slot_index = match rekindle_transport::operations::community::await_join_approval(
        &transport,
        &submitted.registry_key,
        &submitted.our_pseudonym_hex,
        &submitted.community_name,
        Some(notify_rx),
        120,
    )
    .await
    {
        Ok(slot) => {
            tracing::info!(community = %submitted.community_name, slot, "handle_join: phase 2 complete — approved");
            slot
        }
        Err(e) => {
            tracing::warn!(community = %submitted.community_name, error = %e, "handle_join: phase 2 failed");
            ctx.pending_joins.lock().remove(&submitted.governance_key);
            return IpcResponse::error(500, format!("community join failed: {e}"));
        }
    };

    // Clean up pending join entry (may already be removed by handler)
    ctx.pending_joins.lock().remove(&submitted.governance_key);

    // Phase 3: Complete join (read channels + cache MEKs)
    match rekindle_transport::operations::community::complete_join(
        &transport,
        &submitted,
        slot_index,
        &ctx.mek_cache,
        &signing_key,
    )
    .await
    {
        Ok(result) => {
            let membership = rekindle_transport::session::CommunityMembership {
                governance_key: result.governance_key.clone(),
                pseudonym_key: result.our_pseudonym_key.clone(),
                display_name: session.identity.display_name.clone(),
                role_ids: Vec::new(),
                slot_index: result.our_slot_index,
                registry_key: result.registry_key.clone(),
                community_name: result.community_name.clone(),
                slot_seed: Some(result.slot_seed),
                channel_record_keys: std::collections::HashMap::new(),
                community_mailbox_key: result.community_mailbox_key.clone(),
                join_inbox_key: String::new(), // joiners don't operate the inbox
                is_operator: false,
                governance_keypair_label: None,
            };
            {
                let mut guard = ctx.session.write();
                if let Some(ref mut s) = *guard {
                    s.join_community(membership);
                }
            }
            if let Err(e) = ctx.save_session() {
                return e;
            }
            IpcResponse::ok(&serde_json::json!({
                "community_name": result.community_name,
                "governance_key": result.governance_key,
                "channels": result.channels.len(),
                "meks_cached": result.meks_cached,
            }))
        }
        Err(e) => IpcResponse::error(500, format!("community join completion failed: {e}")),
    }
}

pub(crate) async fn handle_leave(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
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

    let signing_key = match ctx.require_signing_key() {
        Ok(k) => k,
        Err(e) => return e,
    };
    match rekindle_transport::operations::community::leave_community(
        &transport,
        &membership,
        &ctx.mek_cache,
        &signing_key,
    )
    .await
    {
        Ok(_) => {
            {
                let mut guard = ctx.session.write();
                if let Some(ref mut s) = *guard {
                    s.leave_community(&membership.governance_key);
                }
            }
            if let Err(e) = ctx.save_session() {
                return e;
            }
            IpcResponse::ok(&serde_json::json!({ "left": membership.governance_key }))
        }
        Err(e) => IpcResponse::error(500, format!("community leave failed: {e}")),
    }
}
