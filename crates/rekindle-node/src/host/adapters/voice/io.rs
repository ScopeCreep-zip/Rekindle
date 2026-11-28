//! Directed voice signaling and media egress on the Host's current roster.

use std::sync::Arc;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_gossip::PeerInfo;
use rekindle_voice::signaling::VoiceSignalingDeps as _;
use rekindle_voice::transport::VoiceTransport;

use super::deps::HostVoiceDeps;
use crate::daemon::gossip_adapter::DaemonGossipAdapter;

pub(super) fn request_media_key(
    deps: &HostVoiceDeps,
    community: &str,
    channel: &str,
    sender: &str,
    index: u64,
) {
    let Some(requester) = deps.my_pseudonym(community) else {
        return;
    };
    let Some(transport) = deps.voice().channel_transport(community, channel) else {
        return;
    };
    let request = CommunityEnvelope::control(ControlPayload::VoiceMediaKeyRequest {
        community_id: community.to_owned(),
        channel_id: channel.to_owned(),
        requester,
        sender: sender.to_owned(),
        key_index: index,
    });
    let deps = deps.clone();
    let sender = sender.to_owned();
    deps.ctx()
        .unlock_scope_or_closed()
        .spawn_or_drop("voice media key request", async move {
            let route = transport
                .lock()
                .await
                .peer_entries()
                .into_iter()
                .find_map(|(peer, route)| (peer == sender).then_some(route));
            if let Some(route) = route {
                deps.call_peer(&route, &request).await;
            }
        });
}

pub(super) fn broadcast_media_capabilities(deps: &HostVoiceDeps, community: &str, channel: &str) {
    let Some(caps) = deps.voice().video_caps.read().clone() else {
        return;
    };
    let envelope = CommunityEnvelope::control(ControlPayload::MediaCapabilities {
        channel_id: channel.to_owned(),
        max_pixel_count: caps.max_pixel_count,
        max_fps: caps.max_fps,
        encode_codecs: caps.encode_codecs,
        decode_codecs: caps.decode_codecs,
        supports_optimize_for_latency: caps.supports_optimize_for_latency,
        supported_scalability_modes: caps.supported_scalability_modes,
    });
    send_to_channel(deps, community, channel, &envelope);
}

pub(super) fn send_to_channel(
    deps: &HostVoiceDeps,
    community: &str,
    channel: &str,
    envelope: &CommunityEnvelope,
) {
    let Some(transport) = deps.voice().channel_transport(community, channel) else {
        return;
    };
    let adapter = Arc::new(DaemonGossipAdapter::new(Arc::clone(deps.ctx())));
    let community = community.to_owned();
    let envelope = envelope.clone();
    deps.ctx()
        .unlock_scope_or_closed()
        .spawn_or_drop("channel peer send", async move {
            let peers = peers(&transport).await;
            if let Err(error) =
                rekindle_gossip::send_to_channel_peers(adapter, &community, &envelope, peers).await
            {
                tracing::warn!(%community, %error, "voice: channel send failed");
            }
        });
}

pub(super) async fn send_to_channel_peers_now(
    deps: &HostVoiceDeps,
    community: &str,
    channel: &str,
    envelope: &CommunityEnvelope,
) {
    let Some(transport) = deps.voice().channel_transport(community, channel) else {
        return;
    };
    let adapter = Arc::new(DaemonGossipAdapter::new(Arc::clone(deps.ctx())));
    if let Err(error) = rekindle_gossip::send_to_channel_peers_now(
        adapter,
        community,
        envelope,
        peers(&transport).await,
    )
    .await
    {
        tracing::warn!(%community, %error, "voice: channel leave send failed");
    }
}

async fn peers(transport: &tokio::sync::Mutex<VoiceTransport>) -> Vec<PeerInfo> {
    transport
        .lock()
        .await
        .peer_entries()
        .into_iter()
        .map(|(pseudonym_key, route_blob)| PeerInfo {
            pseudonym_key,
            route_blob,
        })
        .collect()
}
pub(super) fn log_voice_membership(
    deps: &HostVoiceDeps,
    community_id: &str,
    channel_id: &str,
    joined: bool,
) {
    let ctx = deps.ctx();
    let (Some(owner), Ok(db)) = (
        (ctx.session.read().as_ref()).map(|s| s.identity.public_key_hex.clone()),
        ctx.db.current(),
    ) else {
        return;
    };
    let pseudonym = ctx.my_pseudonym(community_id);
    let (cid, ch) = (community_id.to_string(), channel_id.to_string());
    let at = i64::try_from(ctx.clock.now_ms()).unwrap_or(i64::MAX);
    let fired = db.call_detached(move |conn| {
        let event = rekindle_db::repo::voice_session_events::VoiceSessionEventRow {
            owner_key: &owner,
            community_id: &cid,
            channel_id: &ch,
            member_pseudonym: &pseudonym,
            event_type: if joined { "join" } else { "leave" },
            occurred_at_ms: at,
        };
        if let Err(e) = rekindle_db::repo::voice_session_events::record(conn, &event) {
            tracing::warn!(error = %e, "voice session event not recorded");
        }
    });
    if let Err(e) = fired {
        tracing::warn!(error = %e, "voice session event not queued");
    }
}
