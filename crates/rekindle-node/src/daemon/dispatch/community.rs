//! Community dispatch handlers: Create, Join, Leave, List, Info.

use std::sync::Arc;

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;
use crate::validation;

use super::{DaemonContext, state_error};

pub(crate) async fn handle_create(
    ctx: &Arc<DaemonContext>,
    state: DaemonState,
    name: &str,
    description: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let name = match validation::validate_name(name, "Community") { Ok(n) => n, Err(e) => return e };

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

    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let signing_key = match ctx.require_signing_key() { Ok(k) => k, Err(e) => return e };
    let session = match ctx.require_session(Clone::clone) { Ok(s) => s, Err(e) => return e };

    let desc = if description.is_empty() { None } else { Some(description) };
    match rekindle_transport::operations::community::create_community(
        &transport, &session, &name, desc, &ctx.mek_cache, &signing_key,
    ).await {
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
            if let Err(e) = ctx.save_session() { return e; }

            // Store governance and registry keypairs. These MUST persist —
            // without them, the community cannot process joins or govern.
            if !result.governance_keypair_bytes.is_empty() {
                if let Err(e) = crate::state::keystore::store_governance_keypair(
                    gov_key_short, &result.governance_keypair_bytes,
                ).await {
                    return IpcResponse::error(500, format!(
                        "community created but governance keypair storage failed: {e}. \
                         The community will not function. Delete and recreate."
                    ));
                }
            }
            if !result.registry_keypair_bytes.is_empty() {
                let reg_key_short = if result.registry_key.len() > 12 {
                    &result.registry_key[..12]
                } else {
                    &result.registry_key
                };
                if let Err(e) = crate::state::keystore::store_keypair_bytes(
                    &format!("registry-{reg_key_short}"), &result.registry_keypair_bytes,
                ).await {
                    return IpcResponse::error(500, format!(
                        "community created but registry keypair storage failed: {e}. \
                         The community will not function. Delete and recreate."
                    ));
                }
            }

            // Best-effort encrypted backup of governance + registry keypairs.
            // Recovery path if the OS keyring is lost (migration, container rebuild).
            // Encrypted with the signing key so only the identity owner can recover.
            if let Some(ref sk_handle) = *ctx.signing_key.read() {
                let backup_dir = ctx.session_path.parent().unwrap_or(std::path::Path::new("."));
                let gov_backup = backup_dir.join(format!("governance-backup-{gov_key_short}.enc"));
                let reg_backup = backup_dir.join(format!("registry-backup-{}.enc",
                    if result.registry_key.len() > 12 { &result.registry_key[..12] } else { &result.registry_key }));
                let key = sk_handle.as_bytes();
                if let Err(e) = write_encrypted_backup(&gov_backup, &result.governance_keypair_bytes, key) {
                    tracing::warn!(error = %e, "governance keypair backup failed — keyring is the only copy");
                }
                if let Err(e) = write_encrypted_backup(&reg_backup, &result.registry_keypair_bytes, key) {
                    tracing::warn!(error = %e, "registry keypair backup failed — keyring is the only copy");
                }
            }

            // Establish subscription watches + gossip mesh for the new community.
            // Without this, the creator is deaf to all community events (channel
            // messages, member registry changes, join inbox) until daemon restart.
            {
                let ctx_clone = Arc::clone(ctx);
                let gov_key = result.governance_key.clone();
                let name_clone = name.clone();
                #[allow(clippy::await_holding_lock)]
                tokio::spawn(async move {
                    let membership = {
                        let guard = ctx_clone.session.read();
                        guard.as_ref().and_then(|s| s.communities.get(&gov_key).cloned())
                    };
                    if let Some(membership) = membership {
                        let guard = ctx_clone.subscriptions.read();
                        if let Some(ref mgr) = *guard {
                            mgr.setup_community(&membership).await;
                            tracing::info!(community = %name_clone, "subscription watches established for new community");
                        }
                        drop(guard);
                        let bcast_guard = ctx_clone.broadcast_mgr.read();
                        if let Some(ref bcast_mgr) = *bcast_guard {
                            bcast_mgr.register_mesh(&gov_key);
                            tracing::info!(community = %name_clone, "gossip mesh registered for new community");
                        }
                    }
                });
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
    ctx: &Arc<DaemonContext>,
    state: DaemonState,
    invite: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    if let Err(e) = validation::validate_key(invite, "invite/governance key") { return e; }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let signing_key = match ctx.require_signing_key() { Ok(k) => k, Err(e) => return e };
    let session = match ctx.require_session(Clone::clone) { Ok(s) => s, Err(e) => return e };

    // Phase 1: Submit join request to DHT inbox (non-blocking)
    tracing::info!(governance = invite, "handle_join: phase 1 — submitting join request");
    let submitted = match rekindle_transport::operations::community::submit_join_request(
        &transport, &session, invite, &session.identity.display_name, &signing_key,
    ).await {
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
        &transport, &submitted.registry_key, &submitted.our_pseudonym_hex,
        &submitted.community_name, Some(notify_rx), 120,
    ).await {
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
        &transport, &submitted, slot_index, &ctx.mek_cache, &signing_key,
    ).await {
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
            if let Err(e) = ctx.save_session() { return e; }

            // Establish subscription watches + gossip mesh for the new community.
            // Without this, communities joined after daemon startup are deaf.
            {
                let ctx_clone = Arc::clone(ctx);
                let gov_key = result.governance_key.clone();
                let name = result.community_name.clone();
                #[allow(clippy::await_holding_lock)]
                tokio::spawn(async move {
                    let membership = {
                        let guard = ctx_clone.session.read();
                        guard.as_ref().and_then(|s| s.communities.get(&gov_key).cloned())
                    };
                    if let Some(membership) = membership {
                        let guard = ctx_clone.subscriptions.read();
                        if let Some(ref mgr) = *guard {
                            mgr.setup_community(&membership).await;
                            tracing::info!(community = %name, "subscription watches established post-join");
                        }
                        drop(guard);
                        let bcast_guard = ctx_clone.broadcast_mgr.read();
                        if let Some(ref bcast_mgr) = *bcast_guard {
                            bcast_mgr.register_mesh(&gov_key);
                            tracing::info!(community = %name, "gossip mesh registered post-join");
                        }
                    }
                });
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
    ctx: &Arc<DaemonContext>,
    state: DaemonState,
    governance_key: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(governance_key) { Ok(m) => m, Err(e) => return e };

    let signing_key = match ctx.require_signing_key() { Ok(k) => k, Err(e) => return e };
    match rekindle_transport::operations::community::leave_community(
        &transport, &membership, &ctx.mek_cache, &signing_key,
    ).await {
        Ok(_) => {
            {
                let mut guard = ctx.session.write();
                if let Some(ref mut s) = *guard {
                    s.leave_community(&membership.governance_key);
                }
            }
            if let Err(e) = ctx.save_session() { return e; }
            IpcResponse::ok(&serde_json::json!({ "left": membership.governance_key }))
        }
        Err(e) => IpcResponse::error(500, format!("community leave failed: {e}")),
    }
}

pub(crate) fn handle_list(ctx: &Arc<DaemonContext>, state: DaemonState) -> IpcResponse {
    if !state.can_query() { return state_error(state, "query"); }
    ctx.require_session(|session| {
        let communities: Vec<serde_json::Value> = session.communities.values().map(|m| {
            serde_json::json!({
                "governance_key": m.governance_key,
                "name": m.community_name,
                "description": "",
                "member_count": 0,
                "channel_count": 0,
                "our_pseudonym": m.pseudonym_key,
            })
        }).collect();
        IpcResponse::ok(&communities)
    }).unwrap_or_else(|e| e)
}

pub(crate) async fn handle_info(
    ctx: &Arc<DaemonContext>,
    state: DaemonState,
    governance_key: &str,
) -> IpcResponse {
    if !state.can_query() { return state_error(state, "query"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(governance_key) { Ok(m) => m, Err(e) => return e };
    let query = match transport.query(Arc::clone(&ctx.mek_cache), Arc::clone(&ctx.signal)) {
        Ok(q) => q, Err(e) => return IpcResponse::error(500, format!("query engine: {e}")),
    };
    match query.community_detail(&membership).await {
        Ok(detail) => IpcResponse::ok(&detail),
        Err(e) => IpcResponse::error(500, format!("community detail: {e}")),
    }
}

pub(crate) async fn handle_approve(
    ctx: &Arc<DaemonContext>,
    state: DaemonState,
    governance_key: &str,
    member_pseudonym: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(governance_key) { Ok(m) => m, Err(e) => return e };
    if !membership.is_operator {
        return IpcResponse::error(403, "not an operator for this community");
    }

    let signing_key = match ctx.require_signing_key() { Ok(k) => k, Err(e) => return e };
    let dht = match transport.dht() {
        Ok(d) => d, Err(e) => return IpcResponse::error(500, format!("DHT: {e}")),
    };

    // Read moderation queue
    let mut queue = dht.registry().read_moderation_queue(&membership.registry_key).await.unwrap_or_default();
    let Some(pending) = queue.iter().find(|p| p.requester_pseudonym_hex == member_pseudonym).cloned() else {
        return IpcResponse::error(404, format!("no pending request from {member_pseudonym}"));
    };

    // Register member
    let mut members = dht.registry().read_member_index(&membership.registry_key).await.unwrap_or_default();
    let slot = members.iter().map(|m| m.subkey_index).max().map_or(1, |m| m + 1).max(1);
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
    if let Err(e) = dht.registry().write_member_index(&membership.registry_key, &members).await {
        return IpcResponse::error(500, format!("member registration failed: {e}"));
    }

    // Remove from queue
    queue.retain(|p| p.requester_pseudonym_hex != member_pseudonym);
    let _ = dht.registry().write_moderation_queue(&membership.registry_key, &queue).await;

    // Wrap MEKs for the approved member
    let channels = dht.governance().read_channels(&membership.governance_key).await.unwrap_or_default();
    if let Ok(transfers) = rekindle_transport::operations::mek::wrap_meks_for_member(
        &channels, member_pseudonym, &signing_key, &membership.governance_key, &ctx.mek_cache,
    ) {
        let mut vault = dht.registry().read_mek_vault(&membership.registry_key).await.unwrap_or_default();
        for t in &transfers {
            if let Some(e) = vault.iter_mut().find(|e| e.channel_id == t.channel_id) {
                e.copies.push(rekindle_transport::payload::dht_types::EncryptedMekCopy {
                    target_pseudonym: member_pseudonym.to_string(),
                    encrypted_mek: t.wrapped_mek.clone(),
                });
            }
        }
        let _ = dht.registry().write_mek_vault(&membership.registry_key, &vault).await;
    }

    IpcResponse::ok(&serde_json::json!({ "approved": member_pseudonym, "slot": slot }))
}

pub(crate) async fn handle_reject(
    ctx: &Arc<DaemonContext>,
    state: DaemonState,
    governance_key: &str,
    member_pseudonym: &str,
    reason: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(governance_key) { Ok(m) => m, Err(e) => return e };
    if !membership.is_operator {
        return IpcResponse::error(403, "not an operator for this community");
    }

    let dht = match transport.dht() {
        Ok(d) => d, Err(e) => return IpcResponse::error(500, format!("DHT: {e}")),
    };

    let mut queue = dht.registry().read_moderation_queue(&membership.registry_key).await.unwrap_or_default();
    queue.retain(|p| p.requester_pseudonym_hex != member_pseudonym);
    let _ = dht.registry().write_moderation_queue(&membership.registry_key, &queue).await;

    IpcResponse::ok(&serde_json::json!({ "rejected": member_pseudonym, "reason": reason }))
}

pub(crate) async fn handle_pending_members(
    ctx: &Arc<DaemonContext>,
    state: DaemonState,
    governance_key: &str,
) -> IpcResponse {
    if !state.can_query() { return state_error(state, "query"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(governance_key) { Ok(m) => m, Err(e) => return e };

    let dht = match transport.dht() {
        Ok(d) => d, Err(e) => return IpcResponse::error(500, format!("DHT: {e}")),
    };

    let queue = dht.registry().read_moderation_queue(&membership.registry_key).await.unwrap_or_default();
    IpcResponse::ok(&queue)
}

pub(crate) async fn handle_transfer_ownership(
    ctx: &Arc<DaemonContext>,
    state: DaemonState,
    governance_key: &str,
    new_owner_pseudonym: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(governance_key) { Ok(m) => m, Err(e) => return e };
    if !membership.is_operator {
        return IpcResponse::error(403, "not an operator for this community");
    }

    let dht = match transport.dht() {
        Ok(d) => d, Err(e) => return IpcResponse::error(500, format!("DHT: {e}")),
    };

    // Read and update governance metadata
    let Ok(Some(metadata)) = dht.governance().read_metadata(&membership.governance_key).await else {
        return IpcResponse::error(500, "cannot read governance metadata");
    };

    let mut updated = metadata;
    let old_owner = updated.owner_pseudonym.clone();
    updated.owner_pseudonym = new_owner_pseudonym.to_string();
    updated.operator_pseudonyms.retain(|p| p != &old_owner);
    if !updated.operator_pseudonyms.contains(&new_owner_pseudonym.to_string()) {
        updated.operator_pseudonyms.push(new_owner_pseudonym.to_string());
    }

    if let Err(e) = dht.governance().write_metadata(&membership.governance_key, &updated).await {
        return IpcResponse::error(500, format!("metadata update failed: {e}"));
    }

    // Update local session: current user is no longer operator
    {
        let mut guard = ctx.session.write();
        if let Some(ref mut s) = *guard {
            if let Some(m) = s.communities.get_mut(governance_key) {
                m.is_operator = false;
                m.governance_keypair_label = None;
            }
        }
    }
    if let Err(e) = ctx.save_session() { return e; }

    IpcResponse::ok(&serde_json::json!({
        "transferred": true,
        "old_owner": old_owner,
        "new_owner": new_owner_pseudonym,
    }))
}

fn write_encrypted_backup(path: &std::path::Path, data: &[u8], key: &[u8; 32]) -> anyhow::Result<()> {
    use aes_gcm::{Aes256Gcm, aead::{Aead, KeyInit}, Nonce};
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| anyhow::anyhow!("AES init: {e}"))?;
    let mut nonce_bytes = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, data)
        .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    std::fs::write(path, &output)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
