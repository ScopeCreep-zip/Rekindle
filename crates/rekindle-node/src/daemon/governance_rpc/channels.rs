//! Group B governance ops: join approval/rejection plus channel management.

use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_transport::payload::rpc::{CallResponse, GovernanceOp};
use rekindle_utils::timestamp_ms as now_ms;

use crate::daemon::community_rpc::{get_signing_key, get_transport, require_operator_registry};

use super::{ack, ensure_open, get_node, open_dht, reject};

/// Group B governance ops: join approval/rejection plus channel management
/// (approve-join, reject-join, create/delete/update-channel). Split out of
/// `handle_op_inner` to satisfy clippy::too_many_lines.
pub(super) async fn handle_op_group_b(
    operation: GovernanceOp,
    session: &RwLock<Option<rekindle_transport::Session>>,
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &RwLock<rekindle_transport::crypto::mek::MekCache>,
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
    gov_key: &str,
) -> CallResponse {
    match operation {
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

        // Unreachable: the dispatcher only routes group B ops here.
        _ => unreachable!("handle_op_group_b received an out-of-group operation"),
    }
}
