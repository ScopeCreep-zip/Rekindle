//! Channel dispatch handlers: List, Create, Delete, Update, Send, History.

use std::sync::Arc;

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;
use crate::validation;

use super::{state_error, DaemonContext};

pub(crate) async fn handle_list(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let query = match transport.query(Arc::clone(&ctx.mek_cache)) {
        Ok(q) => q,
        Err(e) => return IpcResponse::error(500, format!("query engine: {e}")),
    };
    match query.list_channels(&membership.governance_key).await {
        Ok(channels) => IpcResponse::ok(&channels),
        Err(e) => IpcResponse::error(500, format!("channel list: {e}")),
    }
}

pub(crate) async fn handle_create(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    name: &str,
    kind: &str,
    category: Option<&str>,
    topic: Option<&str>,
    slowmode_seconds: u32,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let name = match validation::validate_name(name, "Channel") {
        Ok(n) => n,
        Err(e) => return e,
    };
    if let Err(e) = validation::validate_channel_kind(kind) {
        return e;
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };

    match rekindle_transport::operations::channel_admin::create_channel(
        &transport,
        &membership.governance_key,
        &name,
        kind,
        category,
        topic,
        slowmode_seconds,
    )
    .await
    {
        Ok(entry) => IpcResponse::ok(&serde_json::json!({
            "id": entry.id,
            "name": entry.name,
            "kind": format!("{:?}", entry.kind),
        })),
        Err(e) => IpcResponse::error(500, format!("channel create failed: {e}")),
    }
}

pub(crate) async fn handle_delete(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    channel_id: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };

    match rekindle_transport::operations::channel_admin::delete_channel(
        &transport,
        &membership.governance_key,
        channel_id,
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "deleted": channel_id })),
        Err(e) => IpcResponse::error(500, format!("channel delete failed: {e}")),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_update(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    channel_id: &str,
    name: Option<&str>,
    topic: Option<&str>,
    slowmode_seconds: Option<u32>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Some(n) = name {
        if let Err(e) = validation::validate_name(n, "Channel") {
            return e;
        }
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };

    match rekindle_transport::operations::channel_admin::update_channel(
        &transport,
        &membership.governance_key,
        channel_id,
        name,
        topic,
        slowmode_seconds,
    )
    .await
    {
        Ok(entry) => IpcResponse::ok(&serde_json::json!({
            "id": entry.id,
            "name": entry.name,
            "topic": entry.topic,
        })),
        Err(e) => IpcResponse::error(500, format!("channel update failed: {e}")),
    }
}

pub(crate) async fn handle_send(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    channel: &str,
    body: &str,
    reply_to: Option<u64>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = validation::validate_message_body(body) {
        return e;
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };

    // Resolve channel name to channel ID
    let query = match transport.query(Arc::clone(&ctx.mek_cache)) {
        Ok(q) => q,
        Err(e) => return IpcResponse::error(500, format!("query engine: {e}")),
    };
    let detail = match query.community_detail(&membership).await {
        Ok(d) => d,
        Err(e) => return IpcResponse::error(500, format!("community detail: {e}")),
    };
    let Some(ch) = detail
        .channels
        .iter()
        .find(|c| c.name == channel || c.id == channel)
    else {
        return IpcResponse::error(404, format!("channel '{channel}' not found"));
    };
    let channel_id = ch.id.clone();

    // Load existing DhtLog keypair from keyring if this member has
    // previously written to this channel.
    let existing_log_keypair_bytes =
        if let Some(existing_key) = membership.channel_record_keys.get(&channel_id) {
            let short = if existing_key.len() > 12 {
                &existing_key[..12]
            } else {
                existing_key
            };
            let label = format!("channel-log-{short}");
            match crate::state::keystore::load_keypair_bytes(&label).await {
                Ok(Some(bytes)) => Some(bytes),
                _ => None,
            }
        } else {
            None
        };

    // Per-member DhtLog architecture: each member owns their own
    // append-only log per channel. send_message creates the log on first
    // write and returns the keypair for the caller to persist.
    match rekindle_transport::operations::channel::send_message(
        &transport,
        &membership,
        &channel_id,
        body,
        reply_to,
        &ctx.mek_cache,
        existing_log_keypair_bytes.as_deref(),
    )
    .await
    {
        Ok(sent) => {
            let is_new_log = sent.new_log_keypair_bytes.is_some();

            // Store new DhtLog keypair in OS keyring
            if let Some(ref kp_bytes) = sent.new_log_keypair_bytes {
                let short = if sent.member_record_key.len() > 12 {
                    &sent.member_record_key[..12]
                } else {
                    &sent.member_record_key
                };
                let label = format!("channel-log-{short}");
                if let Err(e) = crate::state::keystore::store_keypair_bytes(&label, kp_bytes).await
                {
                    tracing::warn!(error = %e, "channel log keypair keyring store failed");
                }
            }

            // Update session with the new log key
            if is_new_log {
                {
                    let mut guard = ctx.session.write();
                    if let Some(ref mut session) = *guard {
                        if let Some(m) = session.communities.get_mut(&membership.governance_key) {
                            m.channel_record_keys
                                .insert(channel_id.clone(), sent.member_record_key.clone());
                        }
                    }
                }
                if let Err(e) = ctx.save_session() {
                    return e;
                }

                // Register channel record key in member registry so other
                // members can discover it for history reads.
                if membership.is_operator {
                    // Operator: write directly to registry (we have the keypair)
                    if let Ok(transport_clone) = ctx.require_transport() {
                        if let Ok(dht) = transport_clone.dht() {
                            crate::daemon::community_rpc::open_registry_writable(
                                &transport_clone,
                                &membership.registry_key,
                            )
                            .await;
                            let mut members = dht
                                .registry()
                                .read_member_index(&membership.registry_key)
                                .await
                                .unwrap_or_default();
                            if let Some(m) = members
                                .iter_mut()
                                .find(|m| m.pseudonym_key == membership.pseudonym_key)
                            {
                                m.channel_records
                                    .insert(channel_id.clone(), sent.member_record_key.clone());
                                let _ = dht
                                    .registry()
                                    .write_member_index(&membership.registry_key, &members)
                                    .await;
                                tracing::info!(channel = %channel_id, "channel log registered in registry (operator)");
                            }
                        }
                    }
                } else {
                    // Non-operator: send RegisterChannelRecord governance RPC to owner
                    if let Ok(transport_clone) = ctx.require_transport() {
                        if let Ok(signing_key) = ctx.require_signing_key() {
                            if !membership.community_mailbox_key.is_empty() {
                                let gov_key = membership.governance_key.clone();
                                let ps_key = membership.pseudonym_key.clone();
                                let ch_id = channel_id.clone();
                                let rec_key = sent.member_record_key.clone();
                                let mailbox_key = membership.community_mailbox_key.clone();
                                // Read community route from mailbox, send governance RPC
                                if let Ok(dht) = transport_clone.dht() {
                                    if let Ok(Some(route_blob)) =
                                        dht.mailbox().read_community_route(&mailbox_key).await
                                    {
                                        if let Ok(target) =
                                            transport_clone.import_route(&route_blob)
                                        {
                                            let pseudonym = rekindle_transport::crypto::pseudonym::derive_community_pseudonym(
                                                &signing_key, &gov_key,
                                            );
                                            let op = rekindle_transport::payload::rpc::GovernanceRequest {
                                                governance_key: gov_key,
                                                operation: rekindle_transport::payload::rpc::GovernanceOp::RegisterChannelRecord {
                                                    member_pseudonym: ps_key,
                                                    channel_id: ch_id.clone(),
                                                    record_key: rec_key,
                                                },
                                            };
                                            let op_bytes =
                                                postcard::to_stdvec(&op).unwrap_or_default();
                                            let _ = transport_clone.caller().call_with_timeout(
                                                &target,
                                                rekindle_transport::frame::TypeId::CommunityGovOp,
                                                &pseudonym.to_bytes(),
                                                &hex::encode(pseudonym.verifying_key().to_bytes()),
                                                &op_bytes,
                                                std::time::Duration::from_secs(10),
                                            ).await;
                                            tracing::info!(channel = %ch_id, "channel log registered via governance RPC");
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            IpcResponse::ok(&serde_json::json!({
                "message_id": sent.message_id,
                "sequence": sent.sequence,
                "member_record_key": sent.member_record_key,
            }))
        }
        Err(e) => IpcResponse::error(500, format!("send failed: {e}")),
    }
}

pub(crate) async fn handle_history(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    channel: &str,
    limit: u32,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let query = match transport.query(Arc::clone(&ctx.mek_cache)) {
        Ok(q) => q,
        Err(e) => return IpcResponse::error(500, format!("query engine: {e}")),
    };
    // Resolve channel name to UUID for channel_record_keys lookup
    let detail = match query.community_detail(&membership).await {
        Ok(d) => d,
        Err(e) => return IpcResponse::error(500, format!("community detail: {e}")),
    };
    let channel_id = detail
        .channels
        .iter()
        .find(|ch| ch.name == channel || ch.id == channel)
        .map_or(channel, |ch| ch.id.as_str());
    match query
        .channel_history(
            &membership.governance_key,
            channel_id,
            "",
            &membership.registry_key,
            limit as usize,
            &membership.channel_record_keys,
        )
        .await
    {
        Ok(messages) => IpcResponse::ok(&messages),
        Err(e) => IpcResponse::error(500, format!("channel history: {e}")),
    }
}
