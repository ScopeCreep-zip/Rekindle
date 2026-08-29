//! Group A governance ops: channel-record registration plus core moderation.

use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_transport::payload::rpc::{CallResponse, GovernanceOp};
use rekindle_utils::timestamp_ms as now_ms;

use crate::daemon::community_rpc::require_operator_registry;

use super::rekey::rekey_all_channels;
use super::{ack, ensure_open, get_node, open_dht, reject, save};

/// Group A governance ops: channel-record registration plus core moderation
/// (register-record, ban, kick, unban, timeout). Split out of
/// `handle_op_inner` to satisfy clippy::too_many_lines. The dispatcher's match
/// is exhaustive, so the trailing `unreachable!` arm is never reached.
pub(super) async fn handle_op_group_a(
    operation: GovernanceOp,
    sender: Option<&str>,
    session: &RwLock<Option<rekindle_transport::Session>>,
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &RwLock<rekindle_transport::crypto::mek::MekCache>,
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
    session_path: &std::path::Path,
    gov_key: &str,
) -> CallResponse {
    match operation {
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

        // Unreachable: the dispatcher only routes group A ops here.
        _ => unreachable!("handle_op_group_a received an out-of-group operation"),
    }
}
