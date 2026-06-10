//! Phase 14.k — voice presence handlers (join / leave / roster).
//!
//! Implements §10.2 mesh↔MCU auto-switching at the 5+ participant
//! threshold and §10.7 always-MCU stage-channel transport reconcile.

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{
    CommunityEnvelope, ControlPayload, VoiceRosterEntry,
};

use crate::signaling::deps::{CommunityVoiceEvent, VoiceSignalingDeps};
use crate::signaling::stage::reconcile_stage_transport;
use crate::topology;
use crate::transport::VoiceTransport;
use crate::VoiceMode;

pub(super) fn handle_voice_join(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: String,
    route_blob: Vec<u8>,
    display_name: Option<String>,
) {
    let stage_info = deps.stage_channel_info(community_id, &channel_id);
    let is_stage = stage_info.as_ref().is_some_and(|s| s.is_stage);

    // Non-stage channels rotate MEK on membership change (§10.7:
    // stage channels never rotate — anyone may listen; only speakers
    // transmit).
    if !is_stage {
        let deps_rot = Arc::clone(deps);
        let cid = community_id.to_string();
        let ch_id = channel_id.clone();
        let sender = sender_pseudonym.to_string();
        let handle = tokio::spawn(async move {
            deps_rot
                .rotate_voice_mek_for_membership(cid, ch_id, sender, true)
                .await;
        });
        deps.register_background_handle(handle);
    }

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
                &*deps_task,
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
    deps: &dyn VoiceSignalingDeps,
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

    transport
        .lock()
        .await
        .add_peer(&sender_key, &blob, joiner_name.as_deref());

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
        send_confirmed_if_first(deps, community_id, channel_id, &transport).await;
    }

    // §10.6 — the joiner needs our MediaCapabilities and missed any
    // advertisement we made before they arrived. Directed re-advertise
    // to the roster (which now includes them); receivers dedup the
    // identical envelope, so peers that already hold our caps drop it.
    deps.advertise_media_capabilities(community_id, channel_id);

    if is_stage {
        reconcile_stage_transport(deps, community_id, channel_id, &transport, &my_pk).await;
        return;
    }

    maybe_switch_to_mcu(deps, community_id, channel_id, &transport, &my_pk).await;

    let participants = roster_entries(deps, &my_pk, &transport).await;
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
async fn send_confirmed_if_first(
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

/// Handshake leg 2, joiner side: a present member acked our VoiceJoin.
/// The ack carries the acker's identity + route, so it alone is enough
/// to add them to our media roster — then complete leg 3.
pub(super) fn handle_voice_join_ack(
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
            transport
                .lock()
                .await
                .add_peer(&acker, &route_blob, display_name.as_deref());
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
pub(super) fn handle_voice_join_confirmed(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: String,
) {
    if !deps.voice_engine_bound_to(community_id, &channel_id) {
        return;
    }
    deps.emit_event(CommunityVoiceEvent::VoicePeerConfirmed {
        community_id: community_id.to_string(),
        channel_id,
        pseudonym_key: sender_pseudonym.to_string(),
    });
}

/// Re-evaluate the mesh→MCU threshold after the roster grew. Shared by
/// the gossip join path (`voice_join_apply`) and the presence-derived
/// roster reconcile so a backstop-repaired member counts toward the
/// 5-participant election exactly like a gossip join would. No-op for
/// stage channels (stage has its own transport reconcile).
pub(crate) async fn maybe_switch_to_mcu(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
    my_pk: &str,
) {
    let is_stage = deps
        .stage_channel_info(community_id, channel_id)
        .is_some_and(|s| s.is_stage);
    if is_stage {
        return;
    }
    let (peer_count, current_mode) = {
        let t = transport.lock().await;
        (t.peer_count(), t.mode().clone())
    };
    let elected_host = if peer_count >= 4 && matches!(current_mode, VoiceMode::Mesh) {
        let mut candidates = transport.lock().await.peer_keys();
        candidates.push(my_pk.to_string());
        let target = crate::election::channel_target(channel_id);
        crate::election::elect_relay_host(candidates.iter(), &target)
    } else {
        None
    };
    match topology::decide_mode_after_join(
        peer_count,
        &current_mode,
        is_stage,
        elected_host.as_deref(),
    ) {
        topology::ModeDecision::SwitchToMcu { host } => {
            tracing::info!(
                peer_count = peer_count + 1,
                host = %host,
                "auto-electing MCU host (5+ participants)"
            );
            crate::signaling::dispatcher::broadcast_mode_switch(
                deps,
                community_id,
                channel_id,
                "mcu",
                Some(host.clone()),
            );
            transport.lock().await.set_mode(VoiceMode::Mcu {
                host_pseudonym: host.clone(),
            });
            if host == my_pk {
                deps.start_mcu_loop();
            }
        }
        topology::ModeDecision::SwitchToMesh | topology::ModeDecision::NoChange => {}
    }
}

/// Build the roster a present member broadcasts to a joiner. Routes
/// come straight from the transport (the route each peer advertised in
/// its VoiceJoin — authoritative for voice), and SELF is appended with
/// `our_route_blob()` so the joiner learns about the broadcaster too —
/// without self, a member that joined first would never appear in a
/// later joiner's roster.
async fn roster_entries(
    deps: &dyn VoiceSignalingDeps,
    my_pk: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
) -> Vec<VoiceRosterEntry> {
    let peers = transport.lock().await.peer_named_entries();
    build_roster_entries(my_pk, deps.our_route_blob(), deps.my_display_name(), peers)
}

/// Pure roster assembly: map the transport's `(pseudonym, route, name)`
/// peers to entries, then append SELF (deduped) with `my_route` +
/// `my_name`. Self must be present or a member that joined first never
/// appears in a later joiner's roster. Extracted from `roster_entries`
/// so the self-inclusion invariant is unit-testable without a deps mock.
fn build_roster_entries(
    my_pk: &str,
    my_route: Vec<u8>,
    my_name: Option<String>,
    peers: Vec<(String, Vec<u8>, Option<String>)>,
) -> Vec<VoiceRosterEntry> {
    let mut entries: Vec<VoiceRosterEntry> = peers
        .into_iter()
        .map(
            |(pseudonym_key, route_blob, display_name)| VoiceRosterEntry {
                pseudonym_key,
                route_blob,
                muted: false,
                deafened: false,
                display_name,
            },
        )
        .collect();

    if !my_pk.is_empty() && !entries.iter().any(|e| e.pseudonym_key == my_pk) {
        entries.push(VoiceRosterEntry {
            pseudonym_key: my_pk.to_string(),
            route_blob: my_route,
            muted: false,
            deafened: false,
            display_name: my_name,
        });
    }
    entries
}

pub(super) fn handle_voice_leave(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: String,
) {
    let stage_info = deps.stage_channel_info(community_id, &channel_id);
    let is_stage = stage_info.as_ref().is_some_and(|s| s.is_stage);

    if !is_stage {
        let deps_rot = Arc::clone(deps);
        let cid = community_id.to_string();
        let ch_id = channel_id.clone();
        let sender = sender_pseudonym.to_string();
        let handle = tokio::spawn(async move {
            deps_rot
                .rotate_voice_mek_for_membership(cid, ch_id, sender, false)
                .await;
        });
        deps.register_background_handle(handle);
    }

    // §10.6 channel scoping — mirror of the join gate: a leave in a
    // channel we're not bound to must not touch our transport (it
    // could evict a same-pseudonym peer who is still in OUR channel).
    let transport = if deps.voice_engine_bound_to(community_id, &channel_id) {
        deps.transport_handle()
    } else {
        None
    };
    let Some(transport) = transport else {
        deps.emit_event(CommunityVoiceEvent::VoiceLeave {
            community_id: community_id.to_string(),
            channel_id,
            pseudonym_key: sender_pseudonym.to_string(),
        });
        return;
    };

    let my_pk = deps.my_pseudonym(community_id).unwrap_or_default();
    let sender_key = sender_pseudonym.to_string();

    {
        let deps_task = Arc::clone(deps);
        let cid = community_id.to_string();
        let ch_id = channel_id.clone();
        let transport = Arc::clone(&transport);
        let sender_key = sender_key.clone();
        let my_pk = my_pk.clone();
        let handle = tokio::spawn(async move {
            voice_leave_apply(&*deps_task, &cid, &ch_id, transport, sender_key, my_pk).await;
        });
        deps.register_background_handle(handle);
    }

    deps.emit_event(CommunityVoiceEvent::VoiceLeave {
        community_id: community_id.to_string(),
        channel_id,
        pseudonym_key: sender_pseudonym.to_string(),
    });
}

async fn voice_leave_apply(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
    transport: Arc<tokio::sync::Mutex<VoiceTransport>>,
    sender_key: String,
    my_pk: String,
) {
    let is_stage = deps
        .stage_channel_info(community_id, channel_id)
        .is_some_and(|s| s.is_stage);

    let (peer_count, current_mode) = {
        let mut t = transport.lock().await;
        t.remove_peer(&sender_key);
        (t.peer_count(), t.mode().clone())
    };

    if is_stage {
        reconcile_stage_transport(deps, community_id, channel_id, &transport, &my_pk).await;
        return;
    }

    let host_left = matches!(
        current_mode,
        VoiceMode::Mcu { ref host_pseudonym } if *host_pseudonym == sender_key
    );

    let elected_host = if host_left && peer_count >= 4 {
        let mut candidates = transport.lock().await.peer_keys();
        candidates.push(my_pk.clone());
        let target = crate::election::channel_target(channel_id);
        crate::election::elect_relay_host(candidates.iter(), &target)
    } else {
        None
    };

    match topology::decide_mode_after_leave(
        peer_count,
        &current_mode,
        is_stage,
        host_left,
        elected_host.as_deref(),
    ) {
        topology::ModeDecision::SwitchToMesh => {
            tracing::info!(peer_count, "voice mode → mesh");
            transport.lock().await.set_mode(VoiceMode::Mesh);
            deps.stop_mcu_loop().await;
            crate::signaling::dispatcher::broadcast_mode_switch(
                deps,
                community_id,
                channel_id,
                "mesh",
                None,
            );
        }
        topology::ModeDecision::SwitchToMcu { host } => {
            tracing::info!(host = %host, "voice mode → mcu (re-elected after host left)");
            crate::signaling::dispatcher::broadcast_mode_switch(
                deps,
                community_id,
                channel_id,
                "mcu",
                Some(host.clone()),
            );
            transport.lock().await.set_mode(VoiceMode::Mcu {
                host_pseudonym: host.clone(),
            });
            if host == my_pk {
                deps.start_mcu_loop();
            }
        }
        topology::ModeDecision::NoChange => {}
    }
}

pub(super) fn handle_voice_roster(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    channel_id: String,
    participants: Vec<VoiceRosterEntry>,
) {
    // §10.1/§10.5 — surface the full roster to the frontend so a joiner
    // immediately sees everyone already present, decoupled from
    // MEK-decrypt. The packet path only ever revealed members whose
    // audio we could decrypt.
    deps.emit_event(CommunityVoiceEvent::VoiceRoster {
        community_id: community_id.to_string(),
        channel_id: channel_id.clone(),
        participants: participants
            .iter()
            .map(|e| crate::signaling::deps::VoiceRosterParticipant {
                pseudonym_key: e.pseudonym_key.clone(),
                display_name: e.display_name.clone(),
            })
            .collect(),
    });

    // §10.6 channel scoping — only absorb roster routes for the
    // channel our engine is bound to. Rosters for other channels are
    // UI-only; adding their peers would cross-wire two channels'
    // media planes.
    if !deps.voice_engine_bound_to(community_id, &channel_id) {
        return;
    }
    let Some(transport) = deps.transport_handle() else {
        return;
    };
    let deps_task = Arc::clone(deps);
    let cid = community_id.to_string();
    let my_pk = deps.my_pseudonym(community_id).unwrap_or_default();
    let handle = tokio::spawn(async move {
        {
            let mut t = transport.lock().await;
            for entry in &participants {
                if !entry.route_blob.is_empty() && entry.pseudonym_key != my_pk {
                    t.add_peer(
                        &entry.pseudonym_key,
                        &entry.route_blob,
                        entry.display_name.as_deref(),
                    );
                }
            }
        }
        // We (the joiner) just learned the channel roster — directed
        // re-advertise so every present member gets our caps. The
        // session-start advertisement ran against an empty roster.
        deps_task.advertise_media_capabilities(&cid, &channel_id);
        // Roster receipt is leg-2 evidence too: a member built and
        // sent it because they saw our VoiceJoin. Advance + confirm.
        if transport.lock().await.advance_handshake_seen() {
            deps_task.emit_event(CommunityVoiceEvent::VoiceJoinHandshake {
                community_id: cid.clone(),
                channel_id: channel_id.clone(),
                state: "seen".to_string(),
                peer: None,
                display_name: None,
            });
        }
        send_confirmed_if_first(&*deps_task, &cid, &channel_id, &transport).await;
    });
    deps.register_background_handle(handle);
}

#[cfg(test)]
mod tests {
    use super::build_roster_entries;

    #[test]
    fn roster_includes_self_with_empty_transport() {
        // The decoupled-roster fix hinges on this: a present member's
        // broadcast must carry itself, even before any peer is in its
        // transport, or a later joiner never learns about it.
        let entries =
            build_roster_entries("me", vec![1, 2, 3], Some("Me Name".to_string()), vec![]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].pseudonym_key, "me");
        assert_eq!(entries[0].route_blob, vec![1, 2, 3]);
        assert_eq!(entries[0].display_name.as_deref(), Some("Me Name"));
    }

    #[test]
    fn roster_appends_self_after_peers() {
        let peers = vec![("peerA".to_string(), vec![9], Some("Peer A".to_string()))];
        let entries = build_roster_entries("me", vec![1], None, peers);
        assert_eq!(entries.len(), 2);
        let peer_a = entries
            .iter()
            .find(|e| e.pseudonym_key == "peerA")
            .expect("peerA present");
        assert_eq!(peer_a.display_name.as_deref(), Some("Peer A"));
        assert!(entries.iter().any(|e| e.pseudonym_key == "me"));
    }

    #[test]
    fn roster_does_not_duplicate_self_already_in_transport() {
        let peers = vec![("me".to_string(), vec![7], None)];
        let entries = build_roster_entries("me", vec![1], None, peers);
        assert_eq!(entries.len(), 1);
        // transport route is authoritative — self is not re-appended
        assert_eq!(entries[0].route_blob, vec![7]);
    }
}
