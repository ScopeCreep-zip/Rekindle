//! Group B governance ops: channel management.
//!
//! Join approval/rejection used to live here too. Those were coordinator
//! RPCs — a member asked the operator to assign them a registry slot —
//! and v2.0 has no coordinator: the approver writes a governance entry
//! and the joiner claims its own slot. Removing them took the moderation
//! queue, member-index and MEK-vault writes on this path with them.

use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_transport::payload::rpc::{CallResponse, GovernanceOp};

use crate::daemon::community_rpc::{get_transport, require_operator_registry};

use super::{ack, reject};

/// Group B governance ops: create/delete/update-channel. Split out of
/// `handle_op_inner` to satisfy clippy::too_many_lines.
pub(super) async fn handle_op_group_b(
    operation: GovernanceOp,
    session: &RwLock<Option<rekindle_transport::Session>>,
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
    gov_key: &str,
) -> CallResponse {
    match operation {
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
