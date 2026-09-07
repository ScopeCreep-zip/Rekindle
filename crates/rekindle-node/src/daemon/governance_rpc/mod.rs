//! Governance operation RPC handler — all 17 permissioned operations.
//!
//! Every operation validates operator status for the target community,
//! gets transport/DHT access, executes the governance write, and returns.

use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_transport::payload::rpc::{CallResponse, GovernanceOp, GovernanceRequest};

use super::community_rpc::{open_registry_writable, HANDLER_DEADLINE};

mod channels;
mod moderation;
mod rekey;
mod roles;

pub(super) fn get_node(
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
) -> Option<Arc<rekindle_transport::TransportNode>> {
    transport.read().as_ref().map(Arc::clone)
}

pub(super) fn open_dht(
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
) -> Option<rekindle_transport::DhtStore> {
    transport.read().as_ref()?.dht().ok()
}

pub(super) async fn ensure_open(node: &rekindle_transport::TransportNode, gov: &str, reg: &str) {
    if !gov.is_empty() {
        let _ = rekindle_transport::broadcast::dht_writes::open_readonly(node, gov).await;
    }
    if !reg.is_empty() {
        open_registry_writable(node, reg).await;
    }
}

pub(super) fn save(session: &RwLock<Option<rekindle_transport::Session>>, path: &std::path::Path) {
    let guard = session.read();
    if let Some(ref s) = *guard {
        let _ = s.save(path);
    }
}

pub(super) fn ack() -> CallResponse {
    CallResponse::Ack
}
pub(super) fn reject(reason: &str) -> CallResponse {
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

    // Dispatch by category. Each group is handled by its own function to
    // keep every function under clippy::too_many_lines. The match lists every
    // operation explicitly (not `_`) so adding a `GovernanceOp` is a compile
    // error here — exhaustiveness is preserved at this dispatch point.
    match req.operation {
        GovernanceOp::RegisterChannelRecord { .. } => {
            moderation::handle_op_group_a(req.operation, session, transport, gov_key).await
        }
        GovernanceOp::CreateChannel { .. }
        | GovernanceOp::DeleteChannel { .. }
        | GovernanceOp::UpdateChannel { .. } => {
            channels::handle_op_group_b(req.operation, session, transport, gov_key).await
        }
        GovernanceOp::CreateRole { .. }
        | GovernanceOp::UpdateRole { .. }
        | GovernanceOp::DeleteRole { .. }
        | GovernanceOp::AssignRole { .. }
        | GovernanceOp::UnassignRole { .. }
        | GovernanceOp::RotateMek { .. }
        | GovernanceOp::TransferOwnership { .. } => {
            roles::handle_op_group_c(
                req.operation,
                session,
                signing_key,
                mek_cache,
                transport,
                session_path,
                gov_key,
            )
            .await
        }
    }
}
