mod media;

use std::sync::Arc;
use std::time::Instant;

use tauri::AppHandle;

use super::ingest_outcome::{IngestOutcome, ShellFollowUp};
use crate::event_dispatch::WebviewEvent;
use crate::state::AppState;
use crate::state_helpers;

use rekindle_codec::capnp_envelope::{decode_signed_envelope, try_decode_community_envelope};
use rekindle_codec::community::envelope::{verify_envelope, CommunityEnvelope, SignedEnvelope};
use rekindle_codec::ingress::{classify, Ingress};
use rekindle_transport::subscriptions::lanes::IngressItem;
use rekindle_types::id::PseudonymKey;
use rekindle_types::key_format::pseudonym_hex;

// Fully synchronous: the voice fast path is a `try_send` and
// everything else is a queue push — the dispatch loop never blocks on
// an app_message again.
pub fn handle(state: &Arc<AppState>, msg: &veilid_core::VeilidAppMessage) {
    let arrived = tokio::time::Instant::now().into_std();
    let message = msg.message();
    // Arrival over our route proves its inbound path (plan RH1r-c O1).
    if let Some(live) = super::network::route_liveness(state) {
        live.heard_over(msg.route_id().map(ToString::to_string).as_deref());
    }
    // Trace, not info: this runs for every media packet (50+/s per peer)
    // on the serial dispatch loop the audio fast path shares (plan C7.23).
    tracing::trace!(msg_len = message.len(), "app_message received");

    // The shared tag classifier routes media inline. Signature verification,
    // reassembly and delivery of gossip run on its worker.
    match classify(message) {
        Ingress::Media(raw) => on_media(state, raw, arrived),
        Ingress::Direct(_) => state
            .gossip_ingress
            .push(IngressItem::Direct(message.to_vec())),
        Ingress::Gossip(bytes) => match decode_signed_envelope(bytes) {
            Ok(signed) => {
                let is_video = video_payload_channel_from_bytes(&signed.envelope_bytes).is_some();
                state
                    .gossip_ingress
                    .push(IngressItem::Gossip { signed, is_video });
            }
            Err(e) => tracing::debug!(error = %e, "malformed gossip datagram dropped"),
        },
        Ingress::Untagged => {
            tracing::debug!(msg_len = message.len(), "untagged app_message dropped");
        }
    }
}

fn on_media(state: &Arc<AppState>, raw: &[u8], arrived: Instant) {
    if !media::handle_media_tag(state, raw, arrived) {
        tracing::debug!(msg_len = raw.len(), "malformed media datagram dropped");
    }
}

/// Worker-side processing of one queued ingress item: the state-only core
/// (`ingest_item`, plan GI4), then its delivery. The dispatch-loop worker
/// only sees this entry point.
pub(crate) async fn process_ingress_item(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    item: IngressItem,
) {
    ingest_item(state, item)
        .await
        .deliver(app_handle, state)
        .await;
}

/// One queued ingress item, ingested with the state alone: the events to
/// show and the follow-ups that need the Tauri handle (plan GI4).
pub(crate) async fn ingest_item(state: &Arc<AppState>, item: IngressItem) -> IngestOutcome {
    match item {
        IngressItem::Gossip { signed, .. } => gossip_core(state, signed).await,
        IngressItem::Direct(message) => {
            let Some(host) = state.host.current() else {
                tracing::debug!("1:1 message before the host started — dropped");
                return IngestOutcome::default();
            };
            let rekindle_transport::operations::direct::Routed::Unported(opened) =
                host.receive_direct(&message).await
            else {
                return IngestOutcome::default();
            };
            let Ok(pool) = state.db.current() else {
                tracing::debug!("unported 1:1 message: no identity database — dropped");
                return IngestOutcome::default();
            };
            IngestOutcome::follow_up(ShellFollowUp::Unported { pool, opened })
        }
    }
}

async fn gossip_core(state: &Arc<AppState>, signed: SignedEnvelope) -> IngestOutcome {
    let community_id = &signed.community_id;

    // Signature verification BEFORE any state mutation (audit P7-W26).
    // Otherwise an attacker can spam unsigned envelopes with crafted
    // sender_pseudonym + dedup_key triples to fill the dedup cache and
    // suppress real gossip from that sender.
    if let Err(e) = verify_envelope(&signed) {
        // Phase F — promote to structured fields so operators reading
        // `RUST_LOG=rekindle_video=debug,rekindle_transport=debug` get
        // the routing context without grep, AND surface video-related
        // rejections to the frontend as `VideoEnvelopeRejected` so the
        // asymmetric-drop case (Pop!_OS rejects, macOS accepts or vice
        // versa) is observable from frontend signals.
        tracing::warn!(
            target: "rekindle_video::verify",
            reason = %e,
            sender_pseudonym = %signed.sender_pseudonym,
            community_id = %community_id,
            "verify_envelope failed"
        );
        // §10.6 — only surface the rejection toast when the local user
        // is actually in the channel the video payload addresses;
        // members outside the channel must not see (or even be told
        // about) channel media.
        if video_payload_channel_from_bytes(&signed.envelope_bytes)
            .is_some_and(|ch| local_in_channel(state, community_id, &ch))
        {
            if let Some(event) = crate::services::video_adapter::rejection_event(
                state,
                community_id.clone(),
                pseudonym_hex(&signed.sender_pseudonym)
                    .ok()
                    .map(|key| PseudonymKey(key.to_bytes())),
            ) {
                return IngestOutcome::event(WebviewEvent::Community(event));
            }
        }
        return IngestOutcome::default();
    }

    let kind = encoded_envelope_kind(&signed.envelope_bytes);
    let dedup_key = gossip_dedup_key(&signed);
    {
        let mut cache = state.dedup_cache.lock();
        if cache.check_and_insert(community_id, &signed.sender_pseudonym, &dedup_key) {
            tracing::debug!(kind, outcome = "duplicate", "gossip received");
            return IngestOutcome::default();
        }
    }

    let video_channel = video_payload_channel_from_bytes(&signed.envelope_bytes);

    // M10.4 — receiver-side per-sender gossip rate floor (architecture
    // §20.2 line 2585). The 10 msg/s cap is enforced even on senders
    // running modified clients that ignore their own send-side limit.
    // Drop is silent: we do not forward, do not ack, do not log at info
    // level (that would amplify the attack via tracing storms).
    //
    // §10.6 exemption: directed channel media (video fragments arrive
    // at frame rate, well above 10/s) has its own congestion control
    // (the media route's transport feedback, plan E4.3.3) and is exempt
    // — but ONLY when we are actively in the addressed channel. Video
    // for any other channel stays under the floor and is then dropped
    // by the receive gate regardless.
    let in_channel_media = video_channel
        .as_deref()
        .is_some_and(|ch| local_in_channel(state, community_id, ch));
    if !in_channel_media
        && !crate::services::community::receiver_limits::check_gossip_rate(
            state,
            community_id,
            &signed.sender_pseudonym,
        )
    {
        // Trace: a flood must not become a log storm (M10.4 above).
        tracing::trace!(kind, outcome = "rate_limited", "gossip received");
        return IngestOutcome::default();
    }

    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.known_members.insert(signed.sender_pseudonym.clone());

            if let Some(ref mut gossip) = cs.gossip {
                if let Some(member) = gossip.online_members.get_mut(&signed.sender_pseudonym) {
                    member.last_seen = rekindle_utils::timestamp_secs();
                }
                if let Some(member) = gossip.peers.get_mut(&signed.sender_pseudonym) {
                    member.last_seen = rekindle_utils::timestamp_secs();
                }
            }
        }
    }

    // A ring watcher's hint (plan E3.2 S7, GI3) passes the Host's one ingest
    // (the community gate, the member gate, its signature; this ingress
    // metered it above) before it is relayed, never relayed first (register
    // S7l), as the daemon's `dispatch_gossip` orders it.
    if let Ok(Some(CommunityEnvelope::Control(payload))) =
        try_decode_community_envelope(&signed.envelope_bytes)
    {
        if let rekindle_codec::community::envelope::ControlPayload::HeadAdvanced { hint } = *payload
        {
            let passed = state_helpers::host_governance(state).is_ok_and(|host| {
                host.ingest_head_advanced(community_id, &signed.sender_pseudonym, &hint)
            });
            if passed && signed.ttl > 0 {
                gossip_forward(state, &signed);
            }
            return IngestOutcome::default();
        }
    }

    // §10.6 — channel media is directed, never epidemic: video
    // payloads are excluded from gossip forwarding even if a
    // non-compliant sender ships them with ttl > 0, so honest nodes
    // never amplify channel media beyond the roster it was sent to.
    if signed.ttl > 0 && video_channel.is_none() {
        gossip_forward(state, &signed);
    }

    let is_from_self = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.as_ref())
            .is_some_and(|pk| pk == &signed.sender_pseudonym)
    };
    if is_from_self {
        tracing::debug!(kind, outcome = "own", "gossip received");
        IngestOutcome::default()
    } else {
        tracing::debug!(
            kind,
            outcome = "accepted",
            ttl = signed.ttl,
            "gossip received"
        );
        relayed_core(state, signed).await
    }
}

/// The receiver's dedup key: the sender's per-send `message_id` (gossip
/// design, architecture §1171; the daemon's `dispatch_bare_gossip` keys
/// the same way). Relayed copies of one send share it; a deliberate
/// re-send of byte-identical content does not, so a `VoiceLeave` after
/// a rejoin is never dropped as the first leave's duplicate.
fn gossip_dedup_key(signed: &SignedEnvelope) -> String {
    rekindle_codec::dedup::message_dedup_key(signed)
}

/// The encoded envelope's kind for the receive log.
fn encoded_envelope_kind(envelope_bytes: &[u8]) -> &'static str {
    match try_decode_community_envelope(envelope_bytes) {
        Ok(Some(envelope)) => envelope_kind(&envelope),
        Ok(None) => "unknown",
        Err(_) => "undecodable",
    }
}

/// An envelope's kind for the send and receive logs: a variant name,
/// never an id.
pub(crate) fn envelope_kind(envelope: &CommunityEnvelope) -> &'static str {
    use rekindle_codec::community::envelope::ControlPayload as C;
    match envelope {
        CommunityEnvelope::TypingIndicator { .. } => "typing",
        CommunityEnvelope::WatchRelay { .. } => "watch_relay",
        CommunityEnvelope::Control(payload) => match &**payload {
            C::VoiceJoin { .. } => "voice_join",
            C::VoiceJoinAck { .. } => "voice_join_ack",
            C::VoiceJoinConfirmed { .. } => "voice_join_confirmed",
            C::VoiceLeave { .. } => "voice_leave",
            C::VoiceRoster { .. } => "voice_roster",
            C::VoiceModeSwitch { .. } => "voice_mode_switch",
            C::VoiceMute { .. } | C::VoiceDeafen { .. } => "voice_mute_state",
            _ => "control",
        },
    }
}

/// Phase F + §10.6 — if the decoded envelope carries a video control
/// payload (fragment, parity, capabilities, keyframe req, bandwidth,
/// topology, ack), the channel it is addressed to. Used at the
/// verify-failure boundary (toast gating), the §20.2 rate floor
/// (in-channel media exemption), and the forward branch (video is
/// directed, never gossip-relayed). The variant set lives in
/// `rekindle_video::video_payload_channel` so the classification has
/// one definition.
///
/// If decoding fails (which is itself a wire-shape issue) we
/// conservatively return `None` — the rejection is logged via tracing
/// and a video-specific UI toast would be a guess.
fn video_payload_channel_from_bytes(envelope_bytes: &[u8]) -> Option<String> {
    let payload = try_decode_community_envelope(envelope_bytes)
        .ok()
        .flatten()
        .and_then(CommunityEnvelope::into_control)?;
    rekindle_video::video_payload_channel(&payload).map(str::to_string)
}

/// True when the local user's active voice/video session is bound to
/// exactly this (community, channel).
fn local_in_channel(state: &Arc<AppState>, community_id: &str, channel_id: &str) -> bool {
    let ve = state.voice_engine.lock();
    ve.as_ref().is_some_and(|handle| {
        handle.community_id.as_deref() == Some(community_id) && handle.channel_id == channel_id
    })
}

/// Relay `signed` one hop further through the Host's mesh (plan E3.4 S5),
/// the one fan-out on both modes: the roster's live members by their `M`/6
/// routes. Only the TTL changes; the author's signature and id stay.
fn gossip_forward(state: &Arc<AppState>, signed: &SignedEnvelope) {
    let mut forward = signed.clone();
    forward.ttl = forward.ttl.saturating_sub(1);
    if let Ok(host) = crate::state_helpers::host_governance(state) {
        host.forward_signed(forward);
    } else {
        tracing::trace!("gossip forward before the host started — dropped");
    }
}

/// The relayed envelope local dispatch may act on, or `None`. First the
/// Host's one community gate (`HostGovernance::takes_traffic_from`, the
/// daemon's `takes_traffic`): nothing once we were removed, nothing from a
/// departed author, for every arm; a relay keeps its author's signature, so
/// the author is who is judged. Before the Host is up nothing is taken.
///
/// Forward-compat: `try_decode_community_envelope` returns `Ok(None)` for
/// unknown union variants. The signature has already been verified and the
/// envelope already forwarded by `gossip_forward`, so local dispatch skips
/// it.
fn admitted_relay(state: &AppState, signed: &SignedEnvelope) -> Option<CommunityEnvelope> {
    let author = rekindle_types::id::PseudonymKey::from_hex_lossy(&signed.sender_pseudonym);
    let taken = state_helpers::host_governance(state)
        .is_ok_and(|host| host.takes_traffic_from(&signed.community_id, &author));
    if !taken {
        tracing::debug!(
            community = %signed.community_id,
            "relayed envelope from traffic we do not take — dropped"
        );
        return None;
    }
    match try_decode_community_envelope(&signed.envelope_bytes) {
        Ok(Some(envelope)) => Some(envelope),
        Ok(None) => {
            tracing::debug!(
                community = %signed.community_id,
                "relayed envelope has unknown variant — forwarded only"
            );
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "invalid relayed envelope");
            None
        }
    }
}

/// A relayed envelope from another member, after the gossip checks: what
/// the Host ingests itself returns nothing; an admitted Control returns
/// `relayed_control_core`'s outcome (plan GI4).
async fn relayed_core(state: &Arc<AppState>, signed: SignedEnvelope) -> IngestOutcome {
    let Some(envelope) = admitted_relay(state, &signed) else {
        return IngestOutcome::default();
    };
    let community_id = signed.community_id.clone();

    match envelope {
        // Channel typing is the Host's (plan E3.4 S9): the frame names its
        // channel by a tag under the text key only the Host holds, and the
        // Host gates and lapses the start, through the one ingest the
        // daemon's gossip path calls. The envelope carries no Lamport
        // stamp; the transport's gossip dispatch passes 0 as well.
        CommunityEnvelope::TypingIndicator {
            channel_tag,
            ttl_ms,
            pseudonym_key,
        } => {
            if let Ok(host) = state_helpers::host_governance(state) {
                host.ingest_typing(
                    &community_id,
                    &signed.sender_pseudonym,
                    0,
                    &pseudonym_key,
                    &channel_tag,
                    ttl_ms,
                );
            } else {
                tracing::trace!("typing frame before the host started — dropped");
            }
            IngestOutcome::default()
        }
        CommunityEnvelope::Control(payload) => {
            if state.db.current().is_err() {
                tracing::debug!("inbound message: no identity database — dropped");
                return IngestOutcome::default();
            }
            super::control::relayed_control_core(
                state,
                &community_id,
                &signed.sender_pseudonym,
                *payload,
            )
            .await
        }
        CommunityEnvelope::WatchRelay {
            record_key,
            subkey,
            content_hash,
            observer_pseudonym,
        } => {
            // Mutual Aid §14.3: the Host's one ingest (plan GI3, the
            // daemon's `ingest_gossip` calls it too) reads the record,
            // checks the hash and applies the value.
            if let Ok(host) = state_helpers::host_governance(state) {
                host.on_watch_relay(&record_key, subkey, &content_hash, &observer_pseudonym)
                    .await;
            }
            IngestOutcome::default()
        }
    }
}

#[cfg(test)]
mod dedup_tests;
#[cfg(test)]
mod ingest_core_tests;
#[cfg(test)]
mod relay_gate_tests;
