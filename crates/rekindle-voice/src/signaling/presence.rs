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

    let Some(transport) = deps.transport_handle() else {
        deps.emit_event(CommunityVoiceEvent::VoiceJoin {
            community_id: community_id.to_string(),
            channel_id,
            pseudonym_key: sender_pseudonym.to_string(),
            route_blob,
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
        let handle = tokio::spawn(async move {
            voice_join_apply(
                &*deps_task,
                &cid,
                &ch_id,
                transport,
                sender_key,
                blob,
                my_pk,
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
    });
}

async fn voice_join_apply(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
    transport: Arc<tokio::sync::Mutex<VoiceTransport>>,
    sender_key: String,
    blob: Vec<u8>,
    my_pk: String,
) {
    let is_stage = deps
        .stage_channel_info(community_id, channel_id)
        .is_some_and(|s| s.is_stage);

    let (peer_count, current_mode) = {
        let mut t = transport.lock().await;
        if let Err(e) = t.add_peer(&sender_key, &blob) {
            tracing::warn!(peer = %sender_key, error = %e, "failed to add voice peer");
        }
        (t.peer_count(), t.mode().clone())
    };

    if is_stage {
        reconcile_stage_transport(deps, community_id, channel_id, &transport, &my_pk).await;
        return;
    }

    let elected_host = if peer_count >= 4 && matches!(current_mode, VoiceMode::Mesh) {
        let mut candidates = transport.lock().await.peer_keys();
        candidates.push(my_pk.clone());
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

    let participants = roster_entries(deps, community_id, &transport).await;
    if !participants.is_empty() {
        let envelope = CommunityEnvelope::Control(ControlPayload::VoiceRoster {
            channel_id: channel_id.to_string(),
            participants,
        });
        deps.send_to_mesh(community_id, &envelope);
    }
}

async fn roster_entries(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
) -> Vec<VoiceRosterEntry> {
    let online = deps.online_voice_members(community_id);
    let peer_keys = transport.lock().await.peer_keys();
    peer_keys
        .iter()
        .filter_map(|pk| {
            online
                .iter()
                .find(|(pseudonym, _)| pseudonym == pk)
                .map(|(_, blob)| VoiceRosterEntry {
                    pseudonym_key: pk.clone(),
                    route_blob: blob.clone(),
                    muted: false,
                    deafened: false,
                })
        })
        .collect()
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

    let Some(transport) = deps.transport_handle() else {
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
    participants: Vec<VoiceRosterEntry>,
) {
    let Some(transport) = deps.transport_handle() else {
        return;
    };
    let handle = tokio::spawn(async move {
        let mut t = transport.lock().await;
        for entry in participants {
            if !entry.route_blob.is_empty() {
                let _ = t.add_peer(&entry.pseudonym_key, &entry.route_blob);
            }
        }
    });
    deps.register_background_handle(handle);
}
