//! Community queries: list, info.

use std::collections::HashMap;

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
pub(crate) fn handle_list(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
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

    let metadata: HashMap<String, rekindle_transport::CommunityMetaSummary> = memberships
        .iter()
        .filter_map(|m| {
            let gov = ctx.community_runtime.governance_state(&m.governance_key)?;
            let meta = gov.metadata.as_ref()?;
            Some((
                m.governance_key.clone(),
                rekindle_transport::CommunityMetaSummary {
                    name: meta.name.clone(),
                    description: meta.description.clone().unwrap_or_default(),
                    creator_pseudonym: gov
                        .creator
                        .as_ref()
                        .map(|c| hex::encode(c.0))
                        .unwrap_or_default(),
                    created_at: 0,
                },
            ))
        })
        .collect();

    IpcResponse::ok(&rekindle_transport::list_communities(
        &memberships,
        &member_counts,
        &channel_counts,
        &metadata,
    ))
}

pub(crate) fn handle_info(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let membership = match ctx.resolve_community(governance_key) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let member_count = u32::try_from(
        ctx.community_runtime
            .member_count(&membership.governance_key),
    )
    .unwrap_or(u32::MAX);
    let channels = super::super::channel::channel_overviews(ctx, &membership.governance_key);
    let roles = super::super::governance::role_displays(ctx, &membership.governance_key);
    let gov = ctx
        .community_runtime
        .governance_state(&membership.governance_key);
    let meta = gov
        .as_ref()
        .map(|gov| rekindle_transport::CommunityMetaSummary {
            name: gov
                .metadata
                .as_ref()
                .map_or_else(|| membership.community_name.clone(), |m| m.name.clone()),
            description: gov
                .metadata
                .as_ref()
                .and_then(|m| m.description.clone())
                .unwrap_or_default(),
            creator_pseudonym: gov
                .creator
                .as_ref()
                .map(|c| hex::encode(c.0))
                .unwrap_or_default(),
            created_at: 0,
        })
        .unwrap_or_default();

    IpcResponse::ok(&rekindle_transport::community_detail(
        &membership,
        member_count,
        channels,
        roles,
        meta,
    ))
}
