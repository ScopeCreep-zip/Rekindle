//! Mesh→MCU auto-election (§10.2) at the 4+ (5th participant) threshold.

use std::sync::Arc;

use crate::signaling::deps::VoiceSignalingDeps;
use crate::topology;
use crate::transport::VoiceTransport;
use crate::VoiceMode;

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
