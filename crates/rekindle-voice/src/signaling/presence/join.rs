//! Voice join handshake: leg 1 (VoiceJoin), the transport-apply task, and
//! leg 3's "confirmed" send. §10.5 rotation-on-membership-change and
//! §10.6 channel-scoped transport absorption live here.

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use crate::signaling::deps::{CommunityVoiceEvent, VoiceSignalingDeps};
use crate::signaling::stage::reconcile_stage_transport;
use crate::transport::VoiceTransport;

use super::mcu::maybe_switch_to_mcu;
use super::roster::roster_entries;

pub(in crate::signaling) fn handle_voice_join(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: String,
    route_blob: Vec<u8>,
    display_name: Option<String>,
) {
    // §10.5 rotation moved into `voice_join_apply`, gated on the
    // roster ACTUALLY changing: VoiceJoin re-announces (every route
    // refresh re-broadcasts one) used to fire a rotation per envelope
    // — generations climbed with zero membership changes and the two
    // peers' independent rotations collided into same-generation
    // different-key split-brain (field: gen 10→16 in one session with
    // one peer). A re-announce is a route upsert, not a membership
    // change.

    // §10.6 channel scoping — only a transport bound to this exact
    // (community, channel) may absorb the joiner. A join announced
    // for any other channel (or while we're not in voice at all) is
    // pure UI signaling: adding the peer would pollute our roster and
    // leak our channel's media to members of a different channel.
    let transport = if deps.voice_engine_bound_to(community_id, &channel_id) {
        deps.transport_handle()
    } else {
        None
    };
    let Some(transport) = transport else {
        deps.emit_event(CommunityVoiceEvent::VoiceJoin {
            community_id: community_id.to_string(),
            channel_id,
            pseudonym_key: sender_pseudonym.to_string(),
            route_blob,
            display_name,
        });
        return;
    };

    let my_pk = deps.my_pseudonym(community_id).unwrap_or_default();
    let sender_key = sender_pseudonym.to_string();
    let blob = route_blob.clone();

    {
        let deps_task = Arc::clone(deps);
        let cid = community_id.to_string();
        let ch_id = channel_id.clone();
        let transport = Arc::clone(&transport);
        let sender_key = sender_key.clone();
        let blob = blob.clone();
        let my_pk = my_pk.clone();
        let joiner_name = display_name.clone();
        let handle = tokio::spawn(async move {
            voice_join_apply(
                &deps_task,
                &cid,
                &ch_id,
                transport,
                JoinApply {
                    sender_key,
                    blob,
                    joiner_name,
                    my_pk,
                },
            )
            .await;
        });
        deps.register_background_handle(handle);
    }

    deps.emit_event(CommunityVoiceEvent::VoiceJoin {
        community_id: community_id.to_string(),
        channel_id,
        pseudonym_key: sender_pseudonym.to_string(),
        route_blob,
        display_name,
    });
}

/// The joiner's identity as carried by their VoiceJoin, plus our own
/// pseudonym — bundled so `voice_join_apply` stays under the argument
/// budget.
struct JoinApply {
    sender_key: String,
    blob: Vec<u8>,
    joiner_name: Option<String>,
    my_pk: String,
}

async fn voice_join_apply(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    channel_id: &str,
    transport: Arc<tokio::sync::Mutex<VoiceTransport>>,
    join: JoinApply,
) {
    let JoinApply {
        sender_key,
        blob,
        joiner_name,
        my_pk,
    } = join;
    let is_stage = deps
        .stage_channel_info(community_id, channel_id)
        .is_some_and(|s| s.is_stage);

    let (newly_added, remote_count) = {
        let mut t = transport.lock().await;
        let newly = t.add_peer(&sender_key, &blob, joiner_name.as_deref());
        (newly, t.peer_count())
    };
    if newly_added {
        deps.emit_event(CommunityVoiceEvent::VoiceRosterChanged {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            pseudonym_key: sender_key.clone(),
            present: true,
            display_name: joiner_name.clone(),
            remote_count,
        });
        // §10.5 — rotate ONLY on a genuine membership change (this
        // peer was not on the roster). Stage channels never rotate
        // (§10.7: anyone may listen; only speakers transmit).
        if !is_stage {
            let deps_rot = Arc::clone(deps);
            let cid = community_id.to_string();
            let ch_id = channel_id.to_string();
            let sender = sender_key.clone();
            let handle = tokio::spawn(async move {
                deps_rot
                    .rotate_voice_mek_for_membership(cid, ch_id, sender, true)
                    .await;
            });
            deps.register_background_handle(handle);
        }
    }

    // Handshake leg 2 — "seen": directed ack carrying OUR identity +
    // route so the joiner can add us from the ack alone (SimpleX
    // x.grp.mem.intro pattern: the introduction carries the member).
    let ack = CommunityEnvelope::Control(ControlPayload::VoiceJoinAck {
        channel_id: channel_id.to_string(),
        joiner_pseudonym: sender_key.clone(),
        display_name: deps.my_display_name(),
        route_blob: deps.our_route_blob(),
    });
    deps.send_to_channel(community_id, channel_id, &ack);

    // Mutual-join race: if WE are still unseen (joined moments ago and
    // nobody acked yet), this peer's VoiceJoin is itself evidence the
    // channel sees us — count it as our leg 2 and complete our leg 3.
    let mutual_seen = transport.lock().await.advance_handshake_seen();
    if mutual_seen {
        deps.emit_event(CommunityVoiceEvent::VoiceJoinHandshake {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            state: "seen".to_string(),
            peer: Some(sender_key.clone()),
            display_name: joiner_name.clone(),
        });
        send_confirmed_if_first(&**deps, community_id, channel_id, &transport).await;
    }

    // §10.6 — the joiner needs our MediaCapabilities and missed any
    // advertisement we made before they arrived. Directed re-advertise
    // to the roster (which now includes them); receivers dedup the
    // identical envelope, so peers that already hold our caps drop it.
    deps.advertise_media_capabilities(community_id, channel_id);

    if is_stage {
        reconcile_stage_transport(&**deps, community_id, channel_id, &transport, &my_pk).await;
        return;
    }

    maybe_switch_to_mcu(&**deps, community_id, channel_id, &transport, &my_pk).await;

    let participants = roster_entries(&**deps, &my_pk, &transport).await;
    if !participants.is_empty() {
        let envelope = CommunityEnvelope::Control(ControlPayload::VoiceRoster {
            channel_id: channel_id.to_string(),
            participants,
        });
        deps.send_to_mesh(community_id, &envelope);
    }
}

/// Handshake leg 3, joiner side: send VoiceJoinConfirmed exactly once
/// (the transport's state machine gates re-entry) and surface the
/// "connected" state to the frontend. Receivers force a video keyframe
/// (RFC 5104 FIR — a new member needs a full intra to start decoding).
pub(in crate::signaling) async fn send_confirmed_if_first(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
) {
    if !transport.lock().await.advance_handshake_connected() {
        return;
    }
    let confirmed = CommunityEnvelope::Control(ControlPayload::VoiceJoinConfirmed {
        channel_id: channel_id.to_string(),
    });
    deps.send_to_channel(community_id, channel_id, &confirmed);
    deps.emit_event(CommunityVoiceEvent::VoiceJoinHandshake {
        community_id: community_id.to_string(),
        channel_id: channel_id.to_string(),
        state: "connected".to_string(),
        peer: None,
        display_name: None,
    });
    tracing::info!(
        community = %community_id,
        channel = %channel_id,
        "voice join handshake complete (confirmed sent)",
    );
}
