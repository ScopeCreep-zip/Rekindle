//! Group C governance ops: role management, MEK rotation, and ownership
//! transfer.

use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_transport::payload::rpc::{CallResponse, GovernanceOp};

use crate::daemon::community_rpc::{get_transport, require_operator_registry};

use super::{ack, ensure_open, get_node, open_dht, reject, save};

/// Group C governance ops: role management, MEK rotation, and ownership
/// transfer (create/update/delete/assign/unassign-role, rotate-mek,
/// transfer-ownership). Split out of `handle_op_inner` to satisfy
/// clippy::too_many_lines.
pub(super) async fn handle_op_group_c(
    operation: GovernanceOp,
    session: &RwLock<Option<rekindle_transport::Session>>,
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
    session_path: &std::path::Path,
    gov_key: &str,
) -> CallResponse {
    match operation {
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

        // Unreachable: the dispatcher only routes group C ops here.
        _ => unreachable!("handle_op_group_c received an out-of-group operation"),
    }
}
