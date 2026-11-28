use rekindle_types::subscription_events::{PacketDropReason, VoiceEvent, VoiceScope};

use super::PacketDrops;

#[test]
fn drop_reports_identify_the_stage_and_render_the_count() {
    let drops = PacketDrops::default();
    drops.record_ingress();
    drops.record_receive();
    drops.record_receive();
    let scope = VoiceScope::Dm {
        peer_key: "peer".into(),
    };
    let events: Vec<VoiceEvent> = drops.drain(Some(scope.clone())).collect();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[0],
        VoiceEvent::PacketsDropped { reason: PacketDropReason::Ingress, count: 1, notice, .. }
            if notice == "1 voice packet was dropped before playback processing."
    ));
    assert!(matches!(
        &events[1],
        VoiceEvent::PacketsDropped { reason: PacketDropReason::ReceiveLoop, count: 2, notice, .. }
            if notice == "2 voice packets were dropped during playback processing."
    ));
    assert_eq!(drops.total_drops(), 3);
    assert_eq!(drops.drain(Some(scope)).count(), 0);
}
