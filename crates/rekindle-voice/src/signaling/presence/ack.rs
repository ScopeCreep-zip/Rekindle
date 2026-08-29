//! Voice join handshake legs 2 and 3, receiver side: `VoiceJoinAck` and
//! `VoiceJoinConfirmed` handling.

use std::sync::Arc;

use crate::signaling::deps::{CommunityVoiceEvent, VoiceSignalingDeps};

use super::join::send_confirmed_if_first;

/// Handshake leg 2, joiner side: a present member acked our VoiceJoin.
/// The ack carries the acker's identity + route, so it alone is enough
/// to add them to our media roster — then complete leg 3.
pub(in crate::signaling) fn handle_voice_join_ack(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: String,
    joiner_pseudonym: &str,
    display_name: Option<String>,
    route_blob: Vec<u8>,
) {
    let my_pk = deps.my_pseudonym(community_id).unwrap_or_default();
    if joiner_pseudonym != my_pk {
        return; // ack addressed to a different joiner
    }
    if !deps.voice_engine_bound_to(community_id, &channel_id) {
        return;
    }
    let Some(transport) = deps.transport_handle() else {
        return;
    };

    // Surface the acker to the UI exactly like a VoiceJoin would —
    // the joiner may not have seen the acker any other way yet.
    deps.emit_event(CommunityVoiceEvent::VoiceJoin {
        community_id: community_id.to_string(),
        channel_id: channel_id.clone(),
        pseudonym_key: sender_pseudonym.to_string(),
        route_blob: route_blob.clone(),
        display_name: display_name.clone(),
    });

    let deps_task = Arc::clone(deps);
    let cid = community_id.to_string();
    let acker = sender_pseudonym.to_string();
    let handle = tokio::spawn(async move {
        if !route_blob.is_empty() {
            let (newly_added, remote_count) = {
                let mut t = transport.lock().await;
                let newly = t.add_peer(&acker, &route_blob, display_name.as_deref());
                (newly, t.peer_count())
            };
            if newly_added {
                deps_task.emit_event(CommunityVoiceEvent::VoiceRosterChanged {
                    community_id: cid.clone(),
                    channel_id: channel_id.clone(),
                    pseudonym_key: acker.clone(),
                    present: true,
                    display_name: display_name.clone(),
                    remote_count,
                });
            }
        }
        if transport.lock().await.advance_handshake_seen() {
            deps_task.emit_event(CommunityVoiceEvent::VoiceJoinHandshake {
                community_id: cid.clone(),
                channel_id: channel_id.clone(),
                state: "seen".to_string(),
                peer: Some(acker),
                display_name,
            });
        }
        send_confirmed_if_first(&*deps_task, &cid, &channel_id, &transport).await;
    });
    deps.register_background_handle(handle);
}

/// Handshake leg 3, member side: the joiner confirmed it is
/// transport-ready. Surface to the UI (roster entry turns solid) —
/// the video runtime keys its FIR-style keyframe off this event.
pub(in crate::signaling) fn handle_voice_join_confirmed(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: String,
) {
    if !deps.voice_engine_bound_to(community_id, &channel_id) {
        return;
    }
    // A directed confirm reaching us means we are on the sender's
    // channel roster — leg-2 evidence ("we are seen") on par with an
    // ack. Load-bearing for the lost-gossip path: when the original
    // VoiceJoin/Ack exchange died on stale routes and the roster was
    // repaired from presence, the peer's confirm is the first envelope
    // proving bidirectional reachability.
    if let Some(transport) = deps.transport_handle() {
        let deps_task = Arc::clone(deps);
        let cid = community_id.to_string();
        let ch = channel_id.clone();
        let peer = sender_pseudonym.to_string();
        let handle = tokio::spawn(async move {
            if transport.lock().await.advance_handshake_seen() {
                deps_task.emit_event(CommunityVoiceEvent::VoiceJoinHandshake {
                    community_id: cid.clone(),
                    channel_id: ch.clone(),
                    state: "seen".to_string(),
                    peer: Some(peer),
                    display_name: None,
                });
            }
            send_confirmed_if_first(&*deps_task, &cid, &ch, &transport).await;
        });
        deps.register_background_handle(handle);
    }
    deps.emit_event(CommunityVoiceEvent::VoicePeerConfirmed {
        community_id: community_id.to_string(),
        channel_id,
        pseudonym_key: sender_pseudonym.to_string(),
    });
}
