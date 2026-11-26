//! The Host's voice requests (plan E4.1 S5a2): `VoiceJoin` starts the one
//! engine on `HostConfig::audio` through `rekindle_voice`'s join, refused
//! in the Host's words while another session is live (lead ruling
//! 2026-11-26); `VoiceLeave` announces the leave and tears the session
//! down. Leave is always available.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_voice::signaling::MediaKeyOutbox;
use rekindle_voice::AudioPrefs;

use super::deps::HostVoiceDeps;
use super::{events, HostVoice};
use crate::daemon::dispatch::DaemonContext;

impl crate::host::Host {
    /// The Host's voice engine state.
    #[must_use]
    pub fn voice(&self) -> &HostVoice {
        &self.ctx.voice
    }
}

/// The unlocked identity's audio preferences, on `HostConfig::audio`.
async fn audio_prefs(ctx: &DaemonContext) -> AudioPrefs {
    let owner = (ctx.session.read().as_ref()).map(|s| s.identity.public_key_hex.clone());
    let prefs = match (owner, ctx.db.current()) {
        (Some(owner), Ok(db)) => db
            .call(move |conn| rekindle_db::repo::preferences::load(conn, &owner))
            .await
            .map_err(|e| tracing::warn!(error = %e, "voice: preferences unreadable"))
            .ok(),
        _ => None,
    }
    .unwrap_or_default();
    AudioPrefs {
        noise_suppression: prefs.noise_suppression,
        echo_cancellation: prefs.echo_cancellation,
        input_volume: prefs.input_volume,
        output_volume: prefs.output_volume,
        input_device: prefs.input_device,
        output_device: prefs.output_device,
        input_channels: prefs.input_channels,
        io: ctx.voice.io,
    }
}

/// Join voice in `channel` of `community` (its anchor). The same channel
/// again is already joined; any other session live is refused with the
/// ruling's words, never ended.
///
/// # Errors
/// The precedence refusal, or why the session could not start.
pub async fn join(
    ctx: Arc<DaemonContext>,
    community: &str,
    channel: &str,
    muted: bool,
    deafened: bool,
) -> Result<(), String> {
    let prefs = audio_prefs(&ctx).await;
    let deps = Arc::new(HostVoiceDeps::new(Arc::clone(&ctx), prefs));
    let scope = events::scope_of(channel, Some(community));
    match ctx.voice.session() {
        Some(live) if live == scope => return Ok(()),
        Some(_) => {
            return Err(ctx
                .voice
                .start_refusal(|c| deps.community_name(c))
                .unwrap_or_else(|| super::LEAVE_CALL_FIRST.to_owned()))
        }
        None => {}
    }
    rekindle_voice::session::local_controls::join_voice_channel(&deps, channel, Some(community))
        .await
        .map_err(|e| e.to_string())?;
    if muted {
        rekindle_voice::session::local_controls::set_local_mute(&deps, true);
    }
    if deafened {
        rekindle_voice::session::local_controls::set_local_deafen(&deps, true);
    }
    Ok(())
}

/// Leave the live session: the leave is announced, the session torn down.
/// `false` when no session was live (nothing to leave).
pub async fn leave(ctx: Arc<DaemonContext>) -> bool {
    if ctx.voice.session().is_none() {
        return false;
    }
    let prefs = audio_prefs(&ctx).await;
    let deps = Arc::new(HostVoiceDeps::new(ctx, prefs));
    if let Err(e) = rekindle_voice::session::local_controls::leave_voice(&deps).await {
        tracing::warn!(error = %e, "voice leave");
    }
    true
}

/// Ask `sender` (on our roster) for its media key `index` (plan C7.20).
/// The answer arrives as a media-key delivery (the Host's answer path is
/// plan E4.1 S5a2-2).
pub(super) fn request_media_key(
    deps: &HostVoiceDeps,
    community_id: &str,
    channel_id: &str,
    sender: &str,
    index: u64,
) {
    let requester = deps.ctx().my_pseudonym(community_id);
    let (Some(transport), Some(node)) = (
        rekindle_voice::VoiceSessionDeps::current_shared_transport(deps),
        deps.ctx().transport.read().clone(),
    ) else {
        return;
    };
    if requester.is_empty() {
        return;
    }
    let request =
        super::deps::media_key_request(community_id, channel_id, requester, sender, index);
    let Ok(bytes) = rekindle_codec::capnp_envelope::encode_community_envelope(&request) else {
        return;
    };
    let sender = sender.to_string();
    deps.ctx()
        .unlock_scope_or_closed()
        .spawn_or_drop("voice media key request", async move {
            let route = (transport.lock().await.peer_entries().into_iter())
                .find_map(|(peer, route)| (peer == sender).then_some(route));
            let Some(route) = route else {
                tracing::debug!(%sender, "media key request not sent: sender is not on our roster");
                return;
            };
            let result = match node.import_route(&route) {
                Ok(target) => node
                    .caller()
                    .call_community_envelope(&target, &route, bytes)
                    .await
                    .map(drop),
                Err(e) => Err(e),
            };
            if let Err(error) = result {
                tracing::debug!(%sender, %error, "media key request not delivered");
            }
        });
}

/// Our media capabilities to the channel's roster. The Host runs no video
/// engine before plan E4.3, so it advertises no codec: peers send it no
/// video.
pub(super) fn broadcast_media_capabilities(
    deps: &HostVoiceDeps,
    community_id: &str,
    channel_id: &str,
) {
    let envelope = CommunityEnvelope::control(ControlPayload::MediaCapabilities {
        channel_id: channel_id.to_string(),
        max_pixel_count: 0,
        max_fps: 0,
        encode_codecs: Vec::new(),
        decode_codecs: Vec::new(),
        supports_optimize_for_latency: false,
        supported_scalability_modes: Vec::new(),
    });
    let deps = deps.clone();
    let (cid, ch) = (community_id.to_string(), channel_id.to_string());
    deps.ctx()
        .unlock_scope_or_closed()
        .spawn_or_drop("voice media capabilities", async move {
            send_to_channel_peers_now(&deps, &cid, &ch, &envelope).await;
        });
}

/// Send `envelope` to every peer on the session's roster, awaited until
/// each send is handed to the transport (a leave at teardown, plan C7.27).
pub(super) async fn send_to_channel_peers_now(
    deps: &HostVoiceDeps,
    community_id: &str,
    channel_id: &str,
    envelope: &CommunityEnvelope,
) {
    let Some(transport) = rekindle_voice::VoiceSessionDeps::current_shared_transport(deps) else {
        return;
    };
    let peers: Vec<rekindle_gossip::PeerInfo> = (transport.lock().await.peer_entries().into_iter())
        .map(|(pseudonym_key, route_blob)| rekindle_gossip::PeerInfo {
            pseudonym_key,
            route_blob,
        })
        .collect();
    let adapter = crate::daemon::gossip_adapter::DaemonGossipAdapter::new(Arc::clone(deps.ctx()));
    if let Err(error) =
        rekindle_gossip::send_to_channel_peers_now(Arc::new(adapter), community_id, envelope, peers)
            .await
    {
        tracing::debug!(community = %community_id, channel = %channel_id, %error,
            "voice: channel send failed");
    }
}

/// Our voice join or leave in the analytics log.
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

/// The voice session's media-key queue on the Host's outbox `media_key`
/// streams (plan E2.5).
pub(super) struct HostMediaKeyOutbox(pub Arc<DaemonContext>);

impl HostMediaKeyOutbox {
    fn streams(&self) -> Result<crate::host::adapters::outbox::MediaKeyStreams, String> {
        self.0
            .outbox
            .media_keys()
            .ok_or_else(|| "no unlocked outbox".to_string())
    }
}

#[async_trait]
impl MediaKeyOutbox for HostMediaKeyOutbox {
    async fn queue(
        &self,
        session: &str,
        recipient: &str,
        client_id: &str,
        sealed: Vec<u8>,
    ) -> Result<(), String> {
        (self
            .streams()?
            .queue(session, recipient, client_id, sealed)
            .await)
            .map(drop)
            .map_err(|e| e.to_string())
    }

    async fn discard_session(&self, session: &str) -> Result<usize, String> {
        (self.streams()?.discard_session(session).await).map_err(|e| e.to_string())
    }
}
