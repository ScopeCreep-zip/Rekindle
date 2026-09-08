//! Channel dispatch handlers: List, Create, Delete, Update, Send, History.

use std::collections::HashMap;
use std::sync::Arc;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_types::display::ChannelOverviewDisplay;

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;
use crate::validation;

use super::{state_error, DaemonContext};

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
    let Some(gov) = ctx.community_runtime.governance_state(community_id) else {
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
fn resolve_channel(
    ctx: &DaemonContext,
    community_id: &str,
    needle: &str,
) -> Option<(rekindle_types::id::ChannelId, String)> {
    let gov = ctx.community_runtime.governance_state(community_id)?;
    gov.channels.iter().find_map(|(channel_id, channel)| {
        let hex_id = hex::encode(channel_id.0);
        (channel.name == needle || hex_id == needle).then_some((*channel_id, hex_id))
    })
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
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };

    // `topic` and `slowmode_seconds` are not `ChannelCreated` fields —
    // they are separate `ChannelUpdated` concerns, so they are applied
    // as a follow-up entry rather than smuggled into creation.
    let _ = (topic, slowmode_seconds);

    let position = ctx
        .community_runtime
        .governance_state(&membership.governance_key)
        .map_or(0, |gov| {
            gov.channels
                .values()
                .map(|channel| channel.position)
                .max()
                .map_or(0, |max| max.saturating_add(1))
        });
    let category_id = category
        .and_then(|id| hex::decode(id).ok())
        .and_then(|b| <[u8; 16]>::try_from(b).ok())
        .map(rekindle_types::id::CategoryId);

    let adapter = super::adapter(ctx);
    match rekindle_governance_runtime::channels::create_channel(
        &adapter,
        &membership.governance_key,
        rekindle_governance_runtime::channels::NewChannel {
            name: name.clone(),
            channel_type: kind.to_string(),
            category_id,
            position,
            parent_voice_channel_id: None,
        },
    )
    .await
    {
        Ok(created) => IpcResponse::ok(&serde_json::json!({
            "id": created.channel_id_hex,
            "name": name,
            "kind": kind,
            "record_key": created.record_key,
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
    let lamport = adapter.increment_lamport(&membership.governance_key);
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
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "deleted": hex_id })),
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
    if let Some(n) = update.name {
        if let Err(e) = validation::validate_name(n, "Channel") {
            return e;
        }
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let Some((id, hex_id)) = resolve_channel(ctx, &membership.governance_key, channel_id) else {
        return IpcResponse::error(404, format!("channel '{channel_id}' not found"));
    };

    let adapter = super::adapter(ctx);
    let lamport = adapter.increment_lamport(&membership.governance_key);
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

    // Resolve the channel from merged governance rather than a DHT
    // round trip through `community_detail`.
    let Some((_, channel_id)) = resolve_channel(ctx, &membership.governance_key, channel) else {
        return IpcResponse::error(404, format!("channel '{channel}' not found"));
    };

    // Resolve where this write lands. For segment 0 that is the record
    // key `ChannelCreated` carries; beyond it, the first writer from our
    // segment creates the record lazily and announces it with
    // `ChannelSegmentLinked`. Either way the key is merged governance
    // state — nothing has to be registered with anyone.
    let adapter = super::adapter(ctx);
    let channel_record_key = match rekindle_governance_runtime::ensure_channel_segment_record(
        &adapter,
        &membership.governance_key,
        &channel_id,
    )
    .await
    {
        Ok(key) => key,
        Err(e) => return IpcResponse::error(500, format!("no channel record to write to: {e}")),
    };

    // Our writer credential is derived from the shared slot seed, not
    // stored: the seed is the durable secret and the keypair is a pure
    // function of it plus our slot. This is the same derivation the
    // presence write uses, so a member who can publish presence can
    // publish messages.
    let Some(slot_seed) = membership.slot_seed else {
        return IpcResponse::error(
            500,
            "no slot seed for this community — cannot derive a channel writer key",
        );
    };
    let slot_keypair_str = match rekindle_transport::broadcast::dht_writes::derive_slot_keypair_str(
        &slot_seed,
        membership.slot_index,
    ) {
        Ok(kp) => kp,
        Err(e) => return IpcResponse::error(500, format!("derive slot keypair: {e}")),
    };
    let signing_key = match ctx.require_signing_key() {
        Ok(k) => k,
        Err(e) => return e,
    };
    // W26 authorship: the entry is signed with our community pseudonym,
    // which is what stops a peer holding any slot keypair from forging
    // another member's messages.
    let pseudonym_signing_key = rekindle_transport::crypto::pseudonym::derive_community_pseudonym(
        &signing_key,
        &membership.governance_key,
    );

    let target = rekindle_transport::operations::channel::ChannelWriteTarget {
        channel_record_key,
        slot_index: membership.slot_index,
        slot_keypair_str,
    };

    match rekindle_transport::operations::channel::send_message(
        &transport,
        &membership,
        &channel_id,
        body,
        reply_to,
        &ctx.mek_cache,
        &target,
        &pseudonym_signing_key,
    )
    .await
    {
        Ok(sent) => {
            ctx.community_runtime
                .mark_channel_synced(&membership.governance_key, &channel_id);

            // PATH 2 — gossip the notification. The SMPL write above is
            // PATH 1 and authoritative; this is what gives online peers
            // the message in 50–150ms instead of waiting for a watch to
            // fire or the 60s inspect to notice. The desktop's pipeline
            // has always done this; the daemon wrote the record and told
            // nobody, so a daemon member's messages appeared to peers
            // only on their next history read.
            //
            // Carries no ciphertext, deliberately — gossip is
            // unencrypted at the envelope layer, so the notification
            // names the message and peers read the body from the record.
            let notification =
                rekindle_protocol::dht::community::envelope::CommunityEnvelope::MessageNotification {
                    channel_id: channel_id.clone(),
                    message_id: sent.message_id.clone(),
                    author_pseudonym: membership.pseudonym_key.clone(),
                    subkey_index: membership.slot_index,
                    lamport_ts: sent.timestamp,
                    sequence: sent.sequence,
                    content_hash: sent.content_hash.clone(),
                    timestamp: sent.timestamp,
                };
            crate::daemon::gossip::send(&ctx.gossip_tx, &membership.governance_key, &notification);

            IpcResponse::ok(&serde_json::json!({
                "message_id": sent.message_id,
                "timestamp": sent.timestamp,
                "channel_record_key": sent.channel_record_key,
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
    let Some((_, channel_id)) = resolve_channel(ctx, &membership.governance_key, channel) else {
        return IpcResponse::error(404, format!("channel '{channel}' not found"));
    };
    let channel_id = channel_id.as_str();
    // Every segment record holding this channel's messages, straight
    // out of merged governance. Replaces walking the registry's member
    // index for per-member log keys.
    let adapter = super::adapter(ctx);
    let record_keys = rekindle_governance_runtime::channel_record_keys_per_segment(
        &adapter,
        &membership.governance_key,
        channel_id,
    );
    // Author names come from the roster the presence poll materialised.
    let display_names: HashMap<String, String> = ctx
        .community_runtime
        .members(&membership.governance_key)
        .into_iter()
        // A member who never set a name has no row to contribute; the
        // reader falls back to their pseudonym rather than rendering an
        // empty author.
        .filter_map(|(pseudonym, record)| record.display_name.map(|name| (pseudonym, name)))
        .collect();
    match query
        .channel_history(
            &membership.governance_key,
            channel_id,
            &record_keys,
            &display_names,
            limit as usize,
        )
        .await
    {
        Ok(messages) => IpcResponse::ok(&messages),
        Err(e) => IpcResponse::error(500, format!("channel history: {e}")),
    }
}
