//! Voice leave handling: transport removal, MEK rotation-on-departure
//! (§10.5), and MCU-host re-election when the departing peer was hosting.

use std::sync::Arc;

use crate::signaling::deps::{CommunityVoiceEvent, VoiceSignalingDeps};
use crate::signaling::stage::reconcile_stage_transport;
use crate::topology;
use crate::transport::VoiceTransport;
use crate::VoiceMode;

pub(in crate::signaling) fn handle_voice_leave(
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

    let (removed, peer_count, current_mode) = {
        let mut t = transport.lock().await;
        let removed = t.remove_peer(&sender_key);
        (removed, t.peer_count(), t.mode().clone())
    };
    if removed {
        deps.emit_event(CommunityVoiceEvent::VoiceRosterChanged {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            pseudonym_key: sender_key.clone(),
            present: false,
            display_name: None,
            remote_count: peer_count,
        });
    }

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
