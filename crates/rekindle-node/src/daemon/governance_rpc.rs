//! Governance operation RPC handler — all 17 permissioned operations.
//!
//! Every operation validates operator status for the target community,
//! gets transport/DHT access, executes the governance write, and returns.

use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_transport::payload::rpc::{CallResponse, GovernanceOp, GovernanceRequest};

use super::community_rpc::{
    get_signing_key, get_transport, open_registry_writable, require_operator_registry,
    HANDLER_DEADLINE,
};

use rekindle_utils::timestamp_ms as now_ms;

fn get_node(
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
) -> Option<Arc<rekindle_transport::TransportNode>> {
    transport.read().as_ref().map(Arc::clone)
}

fn open_dht(
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
) -> Option<rekindle_transport::DhtStore> {
    transport.read().as_ref()?.dht().ok()
}

async fn ensure_open(node: &rekindle_transport::TransportNode, gov: &str, reg: &str) {
    if !gov.is_empty() {
        let _ = rekindle_transport::broadcast::dht_writes::open_readonly(node, gov).await;
    }
    if !reg.is_empty() {
        open_registry_writable(node, reg).await;
    }
}

fn save(session: &RwLock<Option<rekindle_transport::Session>>, path: &std::path::Path) {
    let guard = session.read();
    if let Some(ref s) = *guard {
        let _ = s.save(path);
    }
}

fn ack() -> CallResponse {
    CallResponse::Ack
}
fn reject(reason: &str) -> CallResponse {
    CallResponse::Rejected {
        reason: reason.into(),
    }
}

// ── Main dispatch ──────────────────────────────────────────────────────

pub(crate) async fn handle_op(
    sender: Option<&str>,
    req: GovernanceRequest,
    session: &RwLock<Option<rekindle_transport::Session>>,
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &RwLock<rekindle_transport::crypto::mek::MekCache>,
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
    session_path: &std::path::Path,
) -> CallResponse {
    if let Ok(response) = tokio::time::timeout(
        HANDLER_DEADLINE,
        handle_op_inner(
            sender,
            req,
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
        tracing::error!("governance op handler exceeded deadline — returning Ack");
        CallResponse::Ack
    }
}

async fn handle_op_inner(
    sender: Option<&str>,
    req: GovernanceRequest,
    session: &RwLock<Option<rekindle_transport::Session>>,
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &RwLock<rekindle_transport::crypto::mek::MekCache>,
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
    session_path: &std::path::Path,
) -> CallResponse {
    let gov_key = &req.governance_key;
    tracing::info!(
        sender = ?sender,
        community = %&gov_key[..16.min(gov_key.len())],
        op = ?std::mem::discriminant(&req.operation),
        "governance op"
    );

    match req.operation {
        // ── Channel record registration ─────────────────────────────
        GovernanceOp::RegisterChannelRecord {
            member_pseudonym,
            channel_id,
            record_key,
        } => {
            let Some(registry_key) = require_operator_registry(session, gov_key) else {
                return ack();
            };
            let Some(dht) = open_dht(transport) else {
                return ack();
            };
            let mut members = dht
                .registry()
                .read_member_index(&registry_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "DHT read failed, using empty");
                    Vec::new()
                });
            if let Some(m) = members
                .iter_mut()
                .find(|m| m.pseudonym_key == member_pseudonym)
            {
                m.channel_records.insert(channel_id.clone(), record_key);
                let _ = dht
                    .registry()
                    .write_member_index(&registry_key, &members)
                    .await;
                tracing::info!(
                    member = %&member_pseudonym[..16.min(member_pseudonym.len())],
                    channel = %channel_id,
                    "channel record registered"
                );
            }
            ack()
        }

        // ── Ban (remove + rekey for forward secrecy) ─────────────────
        GovernanceOp::Ban {
            target_pseudonym,
            reason,
        } => {
            let Some(registry_key) = require_operator_registry(session, gov_key) else {
                return reject("not operator");
            };
            let Some(node) = get_node(transport) else {
                return ack();
            };
            let Some(dht) = open_dht(transport) else {
                return ack();
            };
            ensure_open(&node, gov_key, &registry_key).await;

            let mut bans = dht
                .governance()
                .read_bans(gov_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "DHT read failed, using empty");
                    Vec::new()
                });
            if !bans.iter().any(|b| b.pseudonym_key == target_pseudonym) {
                bans.push(rekindle_transport::payload::dht_types::BanEntry {
                    pseudonym_key: target_pseudonym.clone(),
                    reason,
                    banned_by: sender.unwrap_or("system").to_string(),
                    banned_at: now_ms(),
                });
                let _ = dht.governance().write_bans(gov_key, &bans).await;
            }

            let mut members = dht
                .registry()
                .read_member_index(&registry_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "DHT read failed, using empty");
                    Vec::new()
                });
            members.retain(|m| m.pseudonym_key != target_pseudonym);
            let _ = dht
                .registry()
                .write_member_index(&registry_key, &members)
                .await;

            rekey_all_channels(
                &dht,
                gov_key,
                &registry_key,
                &members,
                signing_key,
                mek_cache,
            )
            .await;
            save(session, session_path);
            tracing::info!(target = %&target_pseudonym[..16.min(target_pseudonym.len())], "banned + rekeyed");
            ack()
        }

        // ── Kick (remove only, no rekey — can rejoin) ────────────────
        GovernanceOp::Kick { target_pseudonym } => {
            let Some(registry_key) = require_operator_registry(session, gov_key) else {
                return reject("not operator");
            };
            let Some(dht) = open_dht(transport) else {
                return ack();
            };
            let mut members = dht
                .registry()
                .read_member_index(&registry_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "DHT read failed, using empty");
                    Vec::new()
                });
            members.retain(|m| m.pseudonym_key != target_pseudonym);
            let _ = dht
                .registry()
                .write_member_index(&registry_key, &members)
                .await;
            tracing::info!(target = %&target_pseudonym[..16.min(target_pseudonym.len())], "kicked");
            ack()
        }

        // ── Unban ────────────────────────────────────────────────────
        GovernanceOp::Unban { target_pseudonym } => {
            if require_operator_registry(session, gov_key).is_none() {
                return ack();
            }
            let Some(node) = get_node(transport) else {
                return ack();
            };
            let Some(dht) = open_dht(transport) else {
                return ack();
            };
            ensure_open(&node, gov_key, "").await;
            let mut bans = dht
                .governance()
                .read_bans(gov_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "DHT read failed, using empty");
                    Vec::new()
                });
            bans.retain(|b| b.pseudonym_key != target_pseudonym);
            let _ = dht.governance().write_bans(gov_key, &bans).await;
            tracing::info!(target = %&target_pseudonym[..16.min(target_pseudonym.len())], "unbanned");
            ack()
        }

        // ── Timeout ──────────────────────────────────────────────────
        GovernanceOp::Timeout {
            target_pseudonym,
            duration_seconds,
            ..
        } => {
            let Some(registry_key) = require_operator_registry(session, gov_key) else {
                return ack();
            };
            let Some(dht) = open_dht(transport) else {
                return ack();
            };
            let mut members = dht
                .registry()
                .read_member_index(&registry_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "DHT read failed, using empty");
                    Vec::new()
                });
            if let Some(m) = members
                .iter_mut()
                .find(|m| m.pseudonym_key == target_pseudonym)
            {
                m.timeout_until = Some(now_ms() + duration_seconds * 1000);
            }
            let _ = dht
                .registry()
                .write_member_index(&registry_key, &members)
                .await;
            tracing::info!(target = %&target_pseudonym[..16.min(target_pseudonym.len())], duration_seconds, "timed out");
            ack()
        }

        // ── Approve join from waiting room ───────────────────────────
        GovernanceOp::ApproveJoin { target_pseudonym } => {
            let Some(registry_key) = require_operator_registry(session, gov_key) else {
                return ack();
            };
            let Some(node) = get_node(transport) else {
                return ack();
            };
            let Some(dht) = open_dht(transport) else {
                return ack();
            };
            ensure_open(&node, gov_key, &registry_key).await;

            let mut queue = dht
                .registry()
                .read_moderation_queue(&registry_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "DHT read failed, using empty");
                    Vec::new()
                });
            let Some(pending) = queue
                .iter()
                .find(|p| p.requester_pseudonym_hex == target_pseudonym)
                .cloned()
            else {
                return reject("not in moderation queue");
            };

            let mut members = dht
                .registry()
                .read_member_index(&registry_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "DHT read failed, using empty");
                    Vec::new()
                });
            let slot = members
                .iter()
                .map(|m| m.subkey_index)
                .max()
                .map_or(1, |m| m + 1)
                .max(1);
            members.push(rekindle_transport::payload::dht_types::MemberSummary {
                pseudonym_key: target_pseudonym.clone(),
                display_name: pending.display_name,
                role_ids: Vec::new(),
                joined_at: now_ms(),
                subkey_index: slot,
                onboarding_complete: true,
                timeout_until: None,
                profile_dht_key: Some(pending.profile_dht_key),
                channel_records: std::collections::HashMap::new(),
            });
            let _ = dht
                .registry()
                .write_member_index(&registry_key, &members)
                .await;

            queue.retain(|p| p.requester_pseudonym_hex != target_pseudonym);
            let _ = dht
                .registry()
                .write_moderation_queue(&registry_key, &queue)
                .await;

            // Wrap MEKs for the approved member
            if let Some(sk) = get_signing_key(signing_key) {
                let channels = dht
                    .governance()
                    .read_channels(gov_key)
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!(error = %e, "DHT read failed, using empty");
                        Vec::new()
                    });
                if let Ok(transfers) = rekindle_transport::operations::mek::wrap_meks_for_member(
                    &channels,
                    &target_pseudonym,
                    &sk,
                    gov_key,
                    mek_cache,
                ) {
                    let mut vault = dht
                        .registry()
                        .read_mek_vault(&registry_key)
                        .await
                        .unwrap_or_else(|e| {
                            tracing::warn!(error = %e, "DHT read failed, using empty");
                            Vec::new()
                        });
                    for t in &transfers {
                        if let Some(e) = vault.iter_mut().find(|e| e.channel_id == t.channel_id) {
                            e.copies.push(
                                rekindle_transport::payload::dht_types::EncryptedMekCopy {
                                    target_pseudonym: target_pseudonym.clone(),
                                    encrypted_mek: t.wrapped_mek.clone(),
                                },
                            );
                        }
                    }
                    let _ = dht.registry().write_mek_vault(&registry_key, &vault).await;
                }
            }
            tracing::info!(target = %&target_pseudonym[..16.min(target_pseudonym.len())], slot, "approved from waiting room");
            ack()
        }

        // ── Reject join ──────────────────────────────────────────────
        GovernanceOp::RejectJoin {
            target_pseudonym,
            reason,
        } => {
            let Some(registry_key) = require_operator_registry(session, gov_key) else {
                return ack();
            };
            let Some(dht) = open_dht(transport) else {
                return ack();
            };
            let mut queue = dht
                .registry()
                .read_moderation_queue(&registry_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "DHT read failed, using empty");
                    Vec::new()
                });
            queue.retain(|p| p.requester_pseudonym_hex != target_pseudonym);
            let _ = dht
                .registry()
                .write_moderation_queue(&registry_key, &queue)
                .await;
            tracing::info!(target = %&target_pseudonym[..16.min(target_pseudonym.len())], reason, "rejected");
            ack()
        }

        // ── Channel management ───────────────────────────────────────
        GovernanceOp::CreateChannel { name, kind, topic } => {
            if require_operator_registry(session, gov_key).is_none() {
                return ack();
            }
            let Some(tn) = get_transport(transport) else {
                return ack();
            };
            match rekindle_transport::operations::channel_admin::create_channel(
                &tn,
                gov_key,
                &name,
                &kind,
                None,
                topic.as_deref(),
                0,
            )
            .await
            {
                Ok(e) => {
                    tracing::info!(channel = %e.name, "channel created");
                    ack()
                }
                Err(e) => reject(&e.to_string()),
            }
        }

        GovernanceOp::DeleteChannel { channel_id } => {
            if require_operator_registry(session, gov_key).is_none() {
                return ack();
            }
            let Some(tn) = get_transport(transport) else {
                return ack();
            };
            match rekindle_transport::operations::channel_admin::delete_channel(
                &tn,
                gov_key,
                &channel_id,
            )
            .await
            {
                Ok(()) => ack(),
                Err(e) => reject(&e.to_string()),
            }
        }

        GovernanceOp::UpdateChannel {
            channel_id,
            name,
            topic,
        } => {
            if require_operator_registry(session, gov_key).is_none() {
                return ack();
            }
            let Some(tn) = get_transport(transport) else {
                return ack();
            };
            match rekindle_transport::operations::channel_admin::update_channel(
                &tn,
                gov_key,
                &channel_id,
                name.as_deref(),
                topic.as_deref(),
                None,
            )
            .await
            {
                Ok(_) => ack(),
                Err(e) => reject(&e.to_string()),
            }
        }

        // ── Role management ──────────────────────────────────────────
        GovernanceOp::CreateRole {
            name,
            permissions,
            color,
            position,
        } => {
            if require_operator_registry(session, gov_key).is_none() {
                return ack();
            }
            let Some(tn) = get_transport(transport) else {
                return ack();
            };
            match rekindle_transport::operations::roles::create_role(
                &tn,
                gov_key,
                &name,
                permissions,
                color,
                position,
            )
            .await
            {
                Ok(_) => ack(),
                Err(e) => reject(&e.to_string()),
            }
        }

        GovernanceOp::UpdateRole {
            role_id,
            name,
            permissions,
            color,
        } => {
            if require_operator_registry(session, gov_key).is_none() {
                return ack();
            }
            let Some(tn) = get_transport(transport) else {
                return ack();
            };
            match rekindle_transport::operations::roles::update_role(
                &tn,
                gov_key,
                role_id,
                name.as_deref(),
                permissions,
                color,
            )
            .await
            {
                Ok(_) => ack(),
                Err(e) => reject(&e.to_string()),
            }
        }

        GovernanceOp::DeleteRole { role_id } => {
            if require_operator_registry(session, gov_key).is_none() {
                return ack();
            }
            let Some(tn) = get_transport(transport) else {
                return ack();
            };
            match rekindle_transport::operations::roles::delete_role(&tn, gov_key, role_id).await {
                Ok(()) => ack(),
                Err(e) => reject(&e.to_string()),
            }
        }

        GovernanceOp::AssignRole {
            member_pseudonym,
            role_id,
        } => {
            let Some(registry_key) = require_operator_registry(session, gov_key) else {
                return ack();
            };
            let Some(tn) = get_transport(transport) else {
                return ack();
            };
            match rekindle_transport::operations::roles::assign_role(
                &tn,
                &registry_key,
                &member_pseudonym,
                role_id,
            )
            .await
            {
                Ok(()) => ack(),
                Err(e) => reject(&e.to_string()),
            }
        }

        GovernanceOp::UnassignRole {
            member_pseudonym,
            role_id,
        } => {
            let Some(registry_key) = require_operator_registry(session, gov_key) else {
                return ack();
            };
            let Some(tn) = get_transport(transport) else {
                return ack();
            };
            match rekindle_transport::operations::roles::unassign_role(
                &tn,
                &registry_key,
                &member_pseudonym,
                role_id,
            )
            .await
            {
                Ok(()) => ack(),
                Err(e) => reject(&e.to_string()),
            }
        }

        // ── MEK rotation ────────────────────────────────��────────────
        GovernanceOp::RotateMek { channel_id } => {
            let Some(registry_key) = require_operator_registry(session, gov_key) else {
                return ack();
            };
            let Some(dht) = open_dht(transport) else {
                return ack();
            };
            let members = dht
                .registry()
                .read_member_index(&registry_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "DHT read failed, using empty");
                    Vec::new()
                });
            // Single-channel rekey: generate new MEK, wrap for all members, write vault
            rekey_channels(
                &dht,
                gov_key,
                &registry_key,
                &[channel_id],
                &members,
                signing_key,
                mek_cache,
            )
            .await;
            save(session, session_path);
            ack()
        }

        // ── Ownership transfer ───────────────────────────────────────
        GovernanceOp::TransferOwnership {
            new_owner_pseudonym,
        } => {
            if require_operator_registry(session, gov_key).is_none() {
                return reject("not operator");
            }
            let Some(node) = get_node(transport) else {
                return reject("transport not started");
            };
            let Some(dht) = open_dht(transport) else {
                return reject("transport not started");
            };
            ensure_open(&node, gov_key, "").await;

            // Read current metadata, update owner
            let metadata = dht.governance().read_metadata(gov_key).await.ok().flatten();
            let Some(mut metadata) = metadata else {
                return reject("cannot read metadata");
            };

            let old_owner = metadata.owner_pseudonym.clone();
            metadata.owner_pseudonym.clone_from(&new_owner_pseudonym);

            // Update operator list: remove old owner, add new owner
            metadata.operator_pseudonyms.retain(|p| p != &old_owner);
            if !metadata.operator_pseudonyms.contains(&new_owner_pseudonym) {
                metadata
                    .operator_pseudonyms
                    .push(new_owner_pseudonym.clone());
            }

            let _ = dht.governance().write_metadata(gov_key, &metadata).await;

            // Update local session: current user is no longer operator
            {
                let mut guard = session.write();
                if let Some(ref mut sess) = *guard {
                    if let Some(m) = sess.communities.get_mut(gov_key) {
                        m.is_operator = false;
                        m.governance_keypair_label = None;
                    }
                }
            }
            save(session, session_path);

            tracing::info!(
                old_owner = %&old_owner[..16.min(old_owner.len())],
                new_owner = %&new_owner_pseudonym[..16.min(new_owner_pseudonym.len())],
                "ownership transferred"
            );
            ack()
        }
    }
}

// ── Rekey implementation ───────────────────────────────────────────────

/// Rekey all channels in a community. Used after ban/leave for forward secrecy.
async fn rekey_all_channels(
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
async fn rekey_channels(
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
