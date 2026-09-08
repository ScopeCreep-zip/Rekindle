//! Community queries: list, info.

use std::collections::HashMap;
use std::sync::Arc;

use rekindle_types::display::CommunityOverview;

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;

use crate::daemon::dispatch::{state_error, DaemonContext};

/// List joined communities with their real metadata.
///
/// This used to hand-roll `serde_json::json!` with `member_count: 0`
/// and `channel_count: 0` hardcoded, while `QueryEngine::list_communities`
/// — which reads the actual governance metadata — sat unwired. The CLI
/// dashboard has been rendering every community as empty as a result.
///
/// Falls back to the session-only shape when the transport is down, so
/// a detached daemon still lists what it has joined rather than
/// erroring.
pub(crate) async fn handle_list(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let memberships = match ctx.require_session(|session| {
        session
            .communities
            .values()
            .cloned()
            .collect::<Vec<rekindle_transport::CommunityMembership>>()
    }) {
        Ok(m) => m,
        Err(e) => return e,
    };

    // From the roster the presence poll materialised, not from a DHT
    // read: every row behind these counts was W26-verified and
    // ban-filtered when it was scanned.
    let member_counts: HashMap<String, u32> = memberships
        .iter()
        .map(|m| {
            (
                m.governance_key.clone(),
                u32::try_from(ctx.community_runtime.member_count(&m.governance_key))
                    .unwrap_or(u32::MAX),
            )
        })
        .collect();

    let Ok(transport) = ctx.require_transport() else {
        return IpcResponse::ok(&session_only_overviews(&memberships, &member_counts));
    };
    let query = match transport.query(Arc::clone(&ctx.mek_cache)) {
        Ok(q) => q,
        Err(e) => return IpcResponse::error(500, format!("query engine: {e}")),
    };
    // Channel counts from merged governance, same reasoning as the
    // member counts above: the v1.0 manifest channels subkey has had no
    // writer since channels became `ChannelCreated` entries.
    let channel_counts: HashMap<String, u32> = memberships
        .iter()
        .map(|m| {
            let n = ctx
                .community_runtime
                .governance_state(&m.governance_key)
                .map_or(0, |gov| {
                    u32::try_from(gov.channels.len()).unwrap_or(u32::MAX)
                });
            (m.governance_key.clone(), n)
        })
        .collect();

    match query
        .list_communities(&memberships, &member_counts, &channel_counts)
        .await
    {
        Ok(overviews) => IpcResponse::ok(&overviews),
        Err(e) => {
            tracing::debug!(error = %e, "community list: DHT metadata unavailable, using session");
            IpcResponse::ok(&session_only_overviews(&memberships, &member_counts))
        }
    }
}

/// The overview we can build without reading the DHT: names come from
/// the persisted membership, channel counts are unknown.
fn session_only_overviews(
    memberships: &[rekindle_transport::CommunityMembership],
    member_counts: &HashMap<String, u32>,
) -> Vec<CommunityOverview> {
    memberships
        .iter()
        .map(|m| CommunityOverview {
            governance_key: m.governance_key.clone(),
            name: m.community_name.clone(),
            description: String::new(),
            member_count: member_counts
                .get(&m.governance_key)
                .copied()
                .unwrap_or_default(),
            channel_count: 0,
            our_pseudonym: m.pseudonym_key.clone(),
        })
        .collect()
}

pub(crate) async fn handle_info(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(governance_key) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let query = match transport.query(Arc::clone(&ctx.mek_cache)) {
        Ok(q) => q,
        Err(e) => return IpcResponse::error(500, format!("query engine: {e}")),
    };
    let member_count = u32::try_from(
        ctx.community_runtime
            .member_count(&membership.governance_key),
    )
    .unwrap_or(u32::MAX);
    let channels = super::super::channel::channel_overviews(ctx, &membership.governance_key);
    let roles = super::super::governance::role_displays(ctx, &membership.governance_key);
    match query
        .community_detail(&membership, member_count, channels, roles)
        .await
    {
        Ok(detail) => IpcResponse::ok(&detail),
        Err(e) => IpcResponse::error(500, format!("community detail: {e}")),
    }
}
