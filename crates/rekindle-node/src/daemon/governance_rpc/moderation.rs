//! Group A governance ops: channel-record registration.
//!
//! Ban / kick / unban / timeout used to live here as coordinator RPCs
//! that mutated the member index and a bespoke bans list. Moderation is
//! now a governance entry written by the moderator and validated by
//! every reader (`rekindle_governance_runtime::moderation`) — one CRDT
//! both tracks merge, instead of two stores that disagreed about who was
//! banned.

use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_transport::payload::rpc::{CallResponse, GovernanceOp};

use crate::daemon::community_rpc::require_operator_registry;

use super::{ack, open_dht};

/// Group A governance ops: channel-record registration. Split out of
/// `handle_op_inner` to satisfy clippy::too_many_lines.
pub(super) async fn handle_op_group_a(
    operation: GovernanceOp,
    session: &RwLock<Option<rekindle_transport::Session>>,
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
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
        // The dispatcher routes only RegisterChannelRecord here, and that
        // match is exhaustive over GovernanceOp, so a new op is a compile
        // error there rather than a silent fall-through here.
        _ => ack(),
    }
}
