use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::channels::CommunityEvent;
use crate::db::DbPool;
use crate::services::message_service;
use crate::state::{AppState, OnlineMember};
use crate::state_helpers;

use rekindle_protocol::capnp_envelope::{
    decode_signed_envelope, encode_signed_envelope, try_decode_community_envelope,
};
use rekindle_protocol::dht::community::envelope::{
    verify_envelope, CommunityEnvelope, ControlPayload, SignedEnvelope,
};

// `_app_handle` kept for dispatch-callback signature symmetry — the
// gossip/legacy processing that used it now runs in the ingress worker
// (`process_ingress_item`).
// Now fully synchronous: the voice fast path is a `try_send` and
// everything else is a queue push — the dispatch loop never blocks on
// an app_message again.
pub fn handle(_app_handle: &AppHandle, state: &Arc<AppState>, msg: &veilid_core::VeilidAppMessage) {
    let message = msg.message().to_vec();
    tracing::info!(msg_len = message.len(), "app_message received");

    if !message.is_empty() && message[0] == b'V' {
        let voice_data = &message[1..];
        match rekindle_voice::transport::VoiceTransport::receive(voice_data) {
            Ok(packet) => {
                let tx = state.voice_packet_tx.read().clone();
                if let Some(tx) = tx {
                    if tx.try_send(packet).is_err() {
                        // W14.4 — was trace! (invisible). Channel
                        // full = real backpressure; surface it.
                        tracing::warn!("voice packet channel full, dropping packet");
                        state
                            .voice_pkt_drops
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                } else {
                    // W14.4 — voice_packet_tx is None. Once W14.1's
                    // permanent-ingress refactor lands this branch
                    // is unreachable. Until then, the lazy-init
                    // race (caller-side post-CallAccept) drops
                    // packets here.
                    tracing::warn!(
                        "voice packet arrived before voice session was set up — dropping (W14.1 will fix)"
                    );
                    state
                        .voice_pkt_drops
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            Err(e) => {
                // Deserialize / signature verify failure. Could be a
                // forged packet or a stale-bytes-on-route artifact.
                tracing::info!(error = %e, "voice packet rejected (deserialize/sig)");
                state
                    .voice_pkt_drops
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        return;
    }

    // Everything below is queued, NOT processed inline: gossip/video
    // processing (sig verify → dedup → reassembly → MEK decrypt → emit)
    // is slow enough that a fragment burst would stall this serial
    // dispatch path and starve the voice fast path above. The worker
    // spawned by `lifecycle::dispatch` drains the queue in FIFO order.
    use crate::services::veilid::ingress_queue::IngressItem;
    if let Ok(signed) = decode_signed_envelope(&message) {
        let is_video = video_payload_channel_from_bytes(&signed.envelope_bytes).is_some();
        state
            .gossip_ingress
            .push(IngressItem::Gossip { signed, is_video });
        return;
    }

    state.gossip_ingress.push(IngressItem::Legacy(message));
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
        IngressItem::Legacy(message) => {
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            message_service::handle_incoming_message(app_handle, state, pool.inner(), &message)
                .await;
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

    let dedup_key = extract_dedup_key(&signed);
    {
        let mut cache = state.dedup_cache.lock();
        if cache.check_and_insert(community_id, &signed.sender_pseudonym, &dedup_key) {
            tracing::trace!(dedup_key = %dedup_key, "gossip dedup: dropping duplicate");
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
    // (FrameAck / BandwidthEstimate / KeyframeRequest) and is exempt
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
        tracing::trace!(
            community = %community_id,
            sender = %signed.sender_pseudonym,
            "gossip rate floor exceeded — dropping silently"
        );
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
    let is_private = is_private_control_payload(&signed.envelope_bytes);
    if signed.ttl > 0 && !is_private && video_channel.is_none() {
        gossip_forward(state, community_id, &signed);
    }

    let is_from_self = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.as_ref())
            .is_some_and(|pk| pk == &signed.sender_pseudonym)
    };
    if !is_from_self {
        handle_relayed_envelope(app_handle, state, signed).await;
    }
}

fn extract_dedup_key(signed: &SignedEnvelope) -> String {
    // Forward-compat: `try_decode_community_envelope` returns `Ok(None)`
    // for unknown union variants. Hash the opaque bytes in that case so
    // dedup still suppresses replays without rejecting unknown messages.
    if let Ok(Some(env)) = try_decode_community_envelope(&signed.envelope_bytes) {
        match env {
            CommunityEnvelope::MessageNotification { ref message_id, .. } => message_id.clone(),
            CommunityEnvelope::TypingIndicator {
                ref channel_id,
                ref pseudonym_key,
            } => {
                let bucket = rekindle_utils::timestamp_secs() / 5;
                format!("typing:{channel_id}:{pseudonym_key}:{bucket}")
            }
            CommunityEnvelope::PresenceUpdate {
                ref pseudonym_key, ..
            } => {
                let bucket = rekindle_utils::timestamp_secs() / 30;
                format!("presence:{pseudonym_key}:{bucket}")
            }
            CommunityEnvelope::Control(_) => envelope_hash(&signed.envelope_bytes),
            CommunityEnvelope::WatchRelay {
                ref record_key,
                subkey,
                ref content_hash,
                ..
            } => format!("watch:{record_key}:{subkey}:{content_hash}"),
        }
    } else {
        envelope_hash(&signed.envelope_bytes)
    }
}

fn envelope_hash(envelope_bytes: &[u8]) -> String {
    use blake2::{digest::consts::U16, Blake2b, Digest};

    let mut h = Blake2b::<U16>::new();
    h.update(envelope_bytes);
    hex::encode(h.finalize())
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
    let Ok(Some(CommunityEnvelope::Control(ref payload))) =
        try_decode_community_envelope(envelope_bytes)
    else {
        return None;
    };
    rekindle_video::video_payload_channel(payload).map(str::to_string)
}

/// True when the local user's active voice/video session is bound to
/// exactly this (community, channel).
fn local_in_channel(state: &Arc<AppState>, community_id: &str, channel_id: &str) -> bool {
    let ve = state.voice_engine.lock();
    ve.as_ref().is_some_and(|handle| {
        handle.community_id.as_deref() == Some(community_id) && handle.channel_id == channel_id
    })
}

pub(crate) fn is_private_control_payload(envelope_bytes: &[u8]) -> bool {
    if let Ok(Some(CommunityEnvelope::Control(ref payload))) =
        try_decode_community_envelope(envelope_bytes)
    {
        matches!(
            payload,
            ControlPayload::JoinAccepted { .. }
                | ControlPayload::SlotKeypairGrant { .. }
                | ControlPayload::AdminKeypairGrant { .. }
                | ControlPayload::SyncResponse { .. }
        )
    } else {
        false
    }
}

fn gossip_forward(state: &Arc<AppState>, community_id: &str, signed: &SignedEnvelope) {
    let mut forward = signed.clone();
    forward.ttl = forward.ttl.saturating_sub(1);
    let signed_bytes = encode_signed_envelope(&forward);

    let Some(rc) = state_helpers::safe_routing_context(state) else {
        return;
    };

    let peers: Vec<Vec<u8>> = {
        let communities = state.communities.read();
        let Some(cs) = communities.get(community_id) else {
            return;
        };
        let Some(ref gossip) = cs.gossip else {
            return;
        };
        gossip
            .peers
            .iter()
            .filter(|(pk, _)| *pk != &signed.sender_pseudonym)
            .map(|(_, m)| m.route_blob.clone())
            .collect()
    };

    if peers.is_empty() {
        return;
    }

    for route_blob in peers {
        let rc = rc.clone();
        let data = signed_bytes.clone();
        tokio::spawn(async move {
            match rc.api().import_remote_private_route(route_blob) {
                Ok(route_id) => {
                    let _ = rc
                        .app_message(veilid_core::Target::RouteId(route_id), data)
                        .await;
                }
                Err(e) => {
                    tracing::trace!(error = %e, "gossip forward: route import failed");
                }
            }
        });
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
        CommunityEnvelope::MessageNotification {
            channel_id,
            message_id,
            sequence,
            subkey_index,
            content_hash,
            ..
        } => {
            let pending = crate::services::community::message_notifications::PendingMessageFetch {
                community_id: community_id.clone(),
                channel_id,
                message_id,
                subkey_index,
                sequence,
                content_hash,
                attempt: 0,
            };
            if let Err(e) =
                crate::services::community::handle_message_notification(app_handle, state, pending)
                    .await
            {
                tracing::debug!(
                    community = %community_id,
                    error = %e,
                    "message notification handling failed"
                );
            }
        }
        CommunityEnvelope::TypingIndicator {
            channel_id,
            pseudonym_key,
        } => {
            crate::event_dispatch::emit_live(
                app_handle,
                "community-event",
                &CommunityEvent::ChannelTyping {
                    community_id,
                    channel_id,
                    pseudonym_key,
                },
            );
        }
        CommunityEnvelope::PresenceUpdate {
            pseudonym_key,
            status,
            game_info,
            route_blob,
        } => {
            {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(&community_id) {
                    cs.known_members.insert(pseudonym_key.clone());
                    if let Some(ref mut gossip) = cs.gossip {
                        if status == "offline" {
                            gossip.online_members.remove(&pseudonym_key);
                            gossip.peers.remove(&pseudonym_key);
                        } else if let Some(ref blob) = route_blob {
                            if !blob.is_empty() {
                                let member = OnlineMember {
                                    route_blob: blob.clone(),
                                    status: status.clone(),
                                    last_seen: rekindle_utils::timestamp_secs(),
                                    ..Default::default()
                                };
                                gossip
                                    .online_members
                                    .insert(pseudonym_key.clone(), member.clone());
                                gossip.peers.insert(pseudonym_key.clone(), member);
                            }
                        }
                    }
                }
            }

            let (game_name, game_id, elapsed_seconds, server_address) = if let Some(gi) = game_info
            {
                (
                    Some(gi.game_name),
                    gi.game_id,
                    gi.elapsed_seconds,
                    gi.server_address,
                )
            } else {
                (None, None, None, None)
            };

            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Presence(
                    rekindle_types::subscription_events::PresenceEvent::CommunityMemberChanged {
                        community: community_id,
                        pseudonym: pseudonym_key,
                        // A gossip presence row states both the status
                        // and the game, so both are observed here.
                        snapshot: rekindle_types::subscription_events::PresenceSnapshot::status(
                            status,
                        )
                        .with_game(
                            rekindle_types::subscription_events::GameActivity::from_parts(
                                game_name,
                                game_id,
                                elapsed_seconds,
                                server_address,
                            ),
                        ),
                    },
                ),
            );
        }
        CommunityEnvelope::Control(payload) => {
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            super::control::handle_relayed_control(
                app_handle,
                state,
                &pool,
                &community_id,
                &signed.sender_pseudonym,
                payload,
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

            let routing_context = state_helpers::safe_routing_context(state);
            if let Some(rc) = routing_context {
                let parsed = record_key.parse::<veilid_core::RecordKey>();
                if let Ok(parsed) = parsed {
                    if let Ok(Some(value)) = rc.get_dht_value(parsed, subkey, true).await {
                        let actual = blake3::hash(value.data()).to_hex().to_string();
                        if actual == content_hash {
                            crate::services::presence_service::handle_value_change(
                                app_handle,
                                state,
                                &record_key,
                                &[subkey],
                                value.data(),
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
}
