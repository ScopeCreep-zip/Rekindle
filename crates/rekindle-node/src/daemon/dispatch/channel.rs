//! Channel dispatch handlers: List, Create, Delete, Update (Send and History
//! are in `channel_message`).

use rekindle_governance_runtime::channels::NewChannel;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_types::display::ChannelOverviewDisplay;

use crate::daemon::DaemonState;
use rekindle_ipc::protocol::{CommunityRequest, IpcResponse};

use super::{state_error, DaemonContext};

/// Route one channel request (the router's arm for them).
pub(crate) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: CommunityRequest,
) -> IpcResponse {
    match request {
        CommunityRequest::ChannelList { community } => handle_list(ctx, state, &community),
        CommunityRequest::ChannelCreate {
            community,
            name,
            kind,
            category,
            topic,
            slowmode_seconds,
            parent_voice_channel,
        } => {
            handle_create(
                ctx,
                state,
                &community,
                NewChannel {
                    name: name.trim().to_owned(),
                    channel_type: kind,
                    category_id: category
                        .as_deref()
                        .and_then(|id| hex::decode(id).ok())
                        .and_then(|b| <[u8; 16]>::try_from(b).ok())
                        .map(rekindle_types::id::CategoryId),
                    // Assigned by `handle_create` from merged governance.
                    position: 0,
                    parent_voice_channel_id: parent_voice_channel
                        .as_deref()
                        .and_then(super::channel_admin::channel_id),
                },
                topic.as_deref(),
                slowmode_seconds,
            )
            .await
        }
        CommunityRequest::ChannelDelete {
            community,
            channel_id,
        } => handle_delete(ctx, state, &community, &channel_id).await,
        CommunityRequest::ChannelUpdate {
            community,
            channel_id,
            name,
            topic,
            slowmode_seconds,
        } => {
            handle_update(
                ctx,
                state,
                &community,
                &channel_id,
                ChannelUpdate {
                    name: name.as_deref(),
                    topic: topic.as_deref(),
                    slowmode_seconds,
                },
            )
            .await
        }
        CommunityRequest::ChannelSend {
            community,
            channel,
            body,
            reply_to,
        } => {
            super::channel_message::handle_send(ctx, state, &community, &channel, &body, reply_to)
                .await
        }
        request @ (CommunityRequest::ChannelRename { .. }
        | CommunityRequest::ChannelMove { .. }
        | CommunityRequest::ChannelReorder { .. }
        | CommunityRequest::ChannelSetTopic { .. }
        | CommunityRequest::ChannelSetForumTags { .. }
        | CommunityRequest::ChannelOverwriteSet { .. }
        | CommunityRequest::ChannelOverwriteDelete { .. }
        | CommunityRequest::CategoryCreate { .. }
        | CommunityRequest::CategoryDelete { .. }
        | CommunityRequest::CategoryRename { .. }
        | CommunityRequest::CategoryReorder { .. }) => {
            super::channel_admin::route(ctx, state, request).await
        }
        other => unreachable!("{} is not a channel request", other.name()),
    }
}

/// List a community's channels from merged governance.
///
/// `ChannelCreated` / `ChannelUpdated` / `ChannelArchived` are the
/// authority. This used to read the governance manifest's v1.0 channels
/// subkey, a community-wide entry `o_cnt: 0` credentials nobody to
/// write, and one that no desktop peer has written to since channels
/// became CRDT entries — so the two tracks could not see each other's
/// channels at all.
pub(crate) fn handle_list(ctx: &DaemonContext, state: DaemonState, community: &str) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    IpcResponse::ok(&channel_overviews(ctx, &membership.governance_key))
}

/// Every live channel, sorted by position then name so two peers list
/// one community identically.
pub(crate) fn channel_overviews(
    ctx: &DaemonContext,
    community_id: &str,
) -> Vec<ChannelOverviewDisplay> {
    let Some(gov) = ctx.communities.governance_state(community_id) else {
        return Vec::new();
    };
    let mut out: Vec<ChannelOverviewDisplay> = gov
        .channels
        .iter()
        .map(|(channel_id, channel)| ChannelOverviewDisplay {
            id: hex::encode(channel_id.0),
            name: channel.name.clone(),
            kind: channel.channel_type.clone(),
            category_id: channel.category_id.map(|c| hex::encode(c.0)),
            topic: channel.topic.clone().unwrap_or_default(),
            // The daemon holds no per-channel MEK generation of its own;
            // `mek_cache` tracks it per (community, channel) and the
            // reader consults that when it decrypts.
            mek_generation: 0,
            // v1.0 per-member DhtLog spine. Nothing writes one any more.
            log_key: None,
            sort_order: u16::try_from(channel.position).unwrap_or(u16::MAX),
            // Held in the merged CRDT state; the CLI could not show a
            // slowmode channel as rate-limited without it.
            slowmode_seconds: channel.slowmode_seconds,
        })
        .collect();
    out.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// Resolve a channel by name or hex id against merged governance.
pub(crate) fn resolve_channel(
    ctx: &DaemonContext,
    community_id: &str,
    needle: &str,
) -> Option<(rekindle_types::id::ChannelId, String)> {
    let gov = ctx.communities.governance_state(community_id)?;
    gov.channels.iter().find_map(|(channel_id, channel)| {
        let hex_id = hex::encode(channel_id.0);
        (channel.name == needle || hex_id == needle).then_some((*channel_id, hex_id))
    })
}

/// Create a channel. `channel.position` is assigned here (after the
/// last channel in merged governance); `topic` and `slowmode_seconds`
/// follow as a `ChannelUpdated` entry.
pub(crate) async fn handle_create(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    mut channel: NewChannel,
    topic: Option<&str>,
    slowmode_seconds: u32,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };

    channel.position = ctx
        .communities
        .governance_state(&membership.governance_key)
        .map_or(0, |gov| {
            gov.channels
                .values()
                .map(|channel| channel.position)
                .max()
                .map_or(0, |max| max.saturating_add(1))
        });
    let (name, kind) = (channel.name.clone(), channel.channel_type.clone());
    let adapter = super::adapter(ctx);
    let created = match rekindle_governance_runtime::channels::create_channel(
        &adapter,
        &membership.governance_key,
        channel,
    )
    .await
    {
        Ok(created) => created,
        Err(e) => return IpcResponse::error(500, format!("channel create failed: {e}")),
    };

    // `topic` and `slowmode_seconds` are not `ChannelCreated` fields —
    // they are separate `ChannelUpdated` concerns, applied as a
    // follow-up entry rather than smuggled into creation. Best-effort:
    // the channel itself already exists even if this second write
    // fails, so a failure here is a warning, not a 500 — the caller has
    // the channel id back and can retry with `channel update` directly.
    if topic.is_some() || slowmode_seconds != 0 {
        if let Some(channel_id) = hex::decode(&created.channel_id_hex)
            .ok()
            .and_then(|b| <[u8; 16]>::try_from(b).ok())
            .map(rekindle_types::id::ChannelId)
        {
            let lamport = match adapter.next_governance_lamport(&membership.governance_key) {
                Ok(l) => l,
                Err(e) => {
                    return IpcResponse::error(500, format!("governance clock: {e}"));
                }
            };
            if let Err(e) = rekindle_governance_runtime::apply::write_entry(
                &adapter,
                &membership.governance_key,
                rekindle_types::governance::GovernanceEntry::ChannelUpdated {
                    channel_id,
                    name: None,
                    topic: topic.map(ToOwned::to_owned),
                    forum_tags: None,
                    position: None,
                    slowmode_seconds: (slowmode_seconds != 0).then_some(slowmode_seconds),
                    nsfw: None,
                    category_id: None,
                    lamport,
                },
            )
            .await
            {
                tracing::warn!(
                    channel = %created.channel_id_hex,
                    error = %e,
                    "channel created, but topic/slowmode follow-up write failed"
                );
            }
        }
    }

    adapter
        .persist_governance_snapshot(&membership.governance_key)
        .await;
    IpcResponse::ok(&serde_json::json!({
        "id": created.channel_id_hex,
        "name": name,
        "kind": kind,
        "record_key": created.record_key,
    }))
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
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let Some((id, hex_id)) = resolve_channel(ctx, &membership.governance_key, channel_id) else {
        return IpcResponse::error(404, format!("channel '{channel_id}' not found"));
    };

    // Archived, not deleted. The channel's SMPL record still holds every
    // message written to it and `delete_dht_record` is local-only, so
    // "delete" is a governance statement about visibility rather than a
    // claim to have removed anything from the network.
    let adapter = super::adapter(ctx);
    let lamport = match adapter.next_governance_lamport(&membership.governance_key) {
        Ok(l) => l,
        Err(e) => return IpcResponse::error(500, format!("governance clock: {e}")),
    };
    match rekindle_governance_runtime::apply::write_entry(
        &adapter,
        &membership.governance_key,
        rekindle_types::governance::GovernanceEntry::ChannelArchived {
            channel_id: id,
            lamport,
        },
    )
    .await
    {
        Ok(()) => {
            adapter
                .persist_governance_snapshot(&membership.governance_key)
                .await;
            IpcResponse::ok(&serde_json::json!({ "deleted": hex_id }))
        }
        Err(e) => IpcResponse::error(500, format!("channel delete failed: {e}")),
    }
}

/// Mutable fields for a channel update request.
///
/// Groups the optional, caller-supplied channel attributes so the dispatch
/// handler threads a single struct instead of three independent `Option`s.
pub(crate) struct ChannelUpdate<'a> {
    pub name: Option<&'a str>,
    pub topic: Option<&'a str>,
    pub slowmode_seconds: Option<u32>,
}

pub(crate) async fn handle_update(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    channel_id: &str,
    update: ChannelUpdate<'_>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let Some((id, hex_id)) = resolve_channel(ctx, &membership.governance_key, channel_id) else {
        return IpcResponse::error(404, format!("channel '{channel_id}' not found"));
    };

    let adapter = super::adapter(ctx);
    let lamport = match adapter.next_governance_lamport(&membership.governance_key) {
        Ok(l) => l,
        Err(e) => return IpcResponse::error(500, format!("governance clock: {e}")),
    };
    match rekindle_governance_runtime::apply::write_entry(
        &adapter,
        &membership.governance_key,
        rekindle_types::governance::GovernanceEntry::ChannelUpdated {
            channel_id: id,
            name: update.name.map(ToOwned::to_owned),
            topic: update.topic.map(ToOwned::to_owned),
            // Untouched fields stay `None`: the merge is LWW *per field*,
            // so echoing current values back would let a stale update
            // clobber a concurrent change to a field the caller never
            // mentioned.
            forum_tags: None,
            position: None,
            slowmode_seconds: update.slowmode_seconds,
            nsfw: None,
            category_id: None,
            lamport,
        },
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({
            "id": hex_id,
            "name": update.name,
            "topic": update.topic,
        })),
        Err(e) => IpcResponse::error(500, format!("channel update failed: {e}")),
    }
}
