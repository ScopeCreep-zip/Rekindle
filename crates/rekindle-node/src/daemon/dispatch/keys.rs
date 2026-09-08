//! Key management dispatch handlers: MekList, MekRotate, MekRequest, PrekeyReplenish.

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;

use super::{state_error, DaemonContext};

pub(crate) fn handle_mek_list(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let snapshot = ctx.mek_cache.read().snapshot(community);
    IpcResponse::ok(&snapshot)
}

/// Ask for a channel's MEK to be replaced.
///
/// Queued rather than performed inline, and reported as queued rather
/// than as a generation: distribution is per-recipient `app_call` and
/// runs on the rotation worker, so a synchronous answer would either
/// block the IPC handler on the slowest peer or lie about what happened.
/// Clients learn the outcome from `CryptoEvent::MekRotated`, the same
/// event a departure-triggered rotation emits.
///
/// The previous implementation wrapped the new key for every member and
/// published the copies into a registry MEK vault subkey. Two problems:
/// `o_cnt: 0` gives nobody a writer credential for that subkey, and
/// `communities-channels.md` says the MEK is *"**never** written to
/// DHT"*.
pub(crate) fn handle_mek_rotate(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    channel: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.resolve_community(community) {
        return e;
    }

    let request = crate::daemon::mek_rotation::MekRotationRequest::manual(community, channel);
    if ctx.mek_rotation_tx.send(request).is_err() {
        return IpcResponse::error(500, "MEK rotation worker is not running");
    }

    IpcResponse::ok(&serde_json::json!({
        "queued": true,
        "community": community,
        "channel": channel,
    }))
}

pub(crate) fn handle_mek_request(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    channel: &str,
    generation: u64,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };

    // Actually broadcast it. This used to call
    // `operations::mek::build_mek_request_payload`, serialize a postcard
    // `GossipPayload` and return `{"gossip_payload_len": N}` — the bytes
    // were never sent, so asking the daemon for a MEK reported a length
    // and did nothing. A member missing a key stayed missing it.
    //
    // `cascade_index: 0` addresses the deterministic top-rank responder
    // (§A3/P1.3). Only that peer replies; the requester re-sends with an
    // incremented index if nobody does, which is the desktop's
    // `spawn_mek_request_with_retry` loop.
    let request = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
        rekindle_protocol::dht::community::envelope::ControlPayload::RequestMEK {
            channel_id: channel.to_string(),
            needed_generation: generation,
            requester_pseudonym: membership.pseudonym_key.clone(),
            cascade_index: 0,
        },
    );
    crate::daemon::gossip::send(&ctx.gossip_tx, &membership.governance_key, &request);

    IpcResponse::ok(&serde_json::json!({
        "requested": true,
        "channel": channel,
        "generation": generation,
    }))
}

pub(crate) async fn handle_prekey_replenish(
    ctx: &DaemonContext,
    state: DaemonState,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let signing_key = match ctx.require_signing_key() {
        Ok(k) => k,
        Err(e) => return e,
    };
    let profile_key = match ctx.require_session(|s| s.identity.profile_dht_key.clone()) {
        Ok(k) => k,
        Err(e) => return e,
    };

    match rekindle_transport::operations::mek::replenish_prekeys(
        &transport,
        &profile_key,
        &signing_key,
    )
    .await
    {
        Ok(count) => IpcResponse::ok(&serde_json::json!({
            "replenished": true,
            "bytes_written": count,
        })),
        Err(e) => IpcResponse::error(500, format!("prekey replenish: {e}")),
    }
}
