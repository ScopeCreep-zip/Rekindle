mod media;

use std::sync::Arc;

use tauri::AppHandle;

use crate::state::AppState;
use crate::state_helpers;

use rekindle_codec::capnp_envelope::{decode_signed_envelope, try_decode_community_envelope};
use rekindle_codec::community::envelope::{verify_envelope, CommunityEnvelope, SignedEnvelope};
use rekindle_codec::ingress::{classify, Ingress};

// Fully synchronous: the voice fast path is a `try_send` and
// everything else is a queue push — the dispatch loop never blocks on
// an app_message again.
pub fn handle(state: &Arc<AppState>, msg: &veilid_core::VeilidAppMessage) {
    let message = msg.message().to_vec();
    // Arrival over our route proves its inbound path (plan RH1r-c O1).
    if let Some(live) = super::network::route_liveness(state) {
        live.heard_over(msg.route_id().map(ToString::to_string).as_deref());
    }
    // Trace, not info: this runs for every media packet (50+/s per peer)
    // on the serial dispatch loop the audio fast path shares (plan C7.23).
    tracing::trace!(msg_len = message.len(), "app_message received");

    // Both media fast paths (audio packets and their receiver
    // reports) live in `media`: verify, `try_send`, return — never
    // any work on this thread, which is serial across every
    // app_message the node receives.
    if media::handle_media_tag(state, &message) {
        return;
    }

    // Everything below is queued, NOT processed inline: gossip/video
    // processing (sig verify → dedup → reassembly → MEK decrypt → emit)
    // is slow enough that a fragment burst would stall this serial
    // dispatch path and starve the voice fast path above. The worker
    // spawned by `lifecycle::dispatch` drains the queue in FIFO order.
    //
    // Routed by the ingress tag (§6), so nothing here trial-decodes: a
    // datagram that is neither media, `'D'` nor `'G'` is dropped (plan E2.3).
    use crate::services::veilid::ingress_queue::IngressItem;
    match classify(&message) {
        Ingress::Direct(_) => state.gossip_ingress.push(IngressItem::Direct(message)),
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

/// Worker-side processing of one queued ingress item. Lives here so the
/// gossip pipeline (`handle_gossip_envelope`) stays private to this
/// module; the dispatch-loop worker only sees this entry point.
pub(crate) async fn process_ingress_item(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    item: crate::services::veilid::ingress_queue::IngressItem,
) {
    use crate::services::veilid::ingress_queue::IngressItem;
    match item {
        IngressItem::Gossip { signed, .. } => {
            handle_gossip_envelope(app_handle, state, signed).await;
        }
        IngressItem::Direct(message) => {
            let Some(host) = state.host.current() else {
                tracing::debug!("1:1 message before the host started — dropped");
                return;
            };
            if let rekindle_transport::operations::direct::Routed::Unported(opened) =
                host.receive_direct(&message).await
            {
                let Ok(pool) = state.db.current() else {
                    tracing::debug!("unported 1:1 message: no identity database — dropped");
                    return;
                };
                super::inbound_unported::handle(app_handle, state, &pool, *opened).await;
            }
        }
    }
}

async fn handle_gossip_envelope(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    signed: SignedEnvelope,
) {
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
        let reason = e.clone();
        tracing::warn!(
            target: "rekindle_video::verify",
            reason = %reason,
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
            crate::services::video_adapter::VideoAdapter::new(state.clone(), app_handle.clone())
                .emit_video_envelope_rejected(
                    community_id.clone(),
                    signed.sender_pseudonym.clone(),
                    reason,
                );
        }
        return;
    }

    let kind = encoded_envelope_kind(&signed.envelope_bytes);
    let dedup_key = gossip_dedup_key(&signed);
    {
        let mut cache = state.dedup_cache.lock();
        if cache.check_and_insert(community_id, &signed.sender_pseudonym, &dedup_key) {
            tracing::debug!(kind, outcome = "duplicate", "gossip received");
            return;
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
        return;
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
    } else {
        tracing::debug!(
            kind,
            outcome = "accepted",
            ttl = signed.ttl,
            "gossip received"
        );
        handle_relayed_envelope(app_handle, state, signed).await;
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

async fn handle_relayed_envelope(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    signed: SignedEnvelope,
) {
    // Forward-compat: `try_decode_community_envelope` returns `Ok(None)`
    // for unknown union variants. The signature has already been
    // verified and the envelope already forwarded by `gossip_forward`,
    // so we simply skip local dispatch in that case.
    let envelope: CommunityEnvelope = match try_decode_community_envelope(&signed.envelope_bytes) {
        Ok(Some(e)) => e,
        Ok(None) => {
            tracing::debug!(
                community = %signed.community_id,
                "relayed envelope has unknown variant — forwarded only"
            );
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "invalid relayed envelope");
            return;
        }
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
        }
        CommunityEnvelope::Control(payload) => {
            if state.db.current().is_err() {
                tracing::debug!("inbound message: no identity database — dropped");
                return;
            }
            super::control::handle_relayed_control(
                app_handle,
                state,
                &community_id,
                &signed.sender_pseudonym,
                *payload,
            )
            .await;
        }
        CommunityEnvelope::WatchRelay {
            record_key,
            subkey,
            content_hash,
            observer_pseudonym,
        } => {
            // Mutual Aid (architecture §14.3 / §11.7): a peer with a live
            // watch slot is telling us a record's subkey changed. Fetch
            // the new value via the existing presence/value pipeline so
            // we don't depend on holding a DHT watch slot ourselves.
            let _ = observer_pseudonym;

            // W11.1 — if we have our OWN active watch on this record, our
            // direct value-change callback will fire (or has already
            // fired) for the same change. Skipping the relay fetch here
            // saves a redundant `get_dht_value` round-trip per gossip
            // notice in the steady state where most members hold their
            // own watch slots. Members who couldn't get a slot still
            // fall through to the fetch path below.
            let we_already_watch = {
                let communities = state.communities.read();
                communities
                    .values()
                    .any(|cs| cs.watched_records.contains(&record_key))
            };
            if we_already_watch {
                tracing::trace!(
                    record_key = %record_key,
                    subkey,
                    "WatchRelay: skipping fetch — own watch covers this record"
                );
                return;
            }

            if let Ok(pool) = state_helpers::record_pool(state) {
                if let Ok(Some(value)) = pool.read_once(&record_key, subkey, true).await {
                    let actual = blake3::hash(&value.data).to_hex().to_string();
                    if actual == content_hash {
                        state_helpers::friend_presence_change(
                            state,
                            &record_key,
                            &[subkey],
                            &value.data,
                        );
                    } else {
                        tracing::debug!(
                            record_key = %record_key,
                            subkey,
                            "WatchRelay content hash mismatch — dropping"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod dedup_tests;
