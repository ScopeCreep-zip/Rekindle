use super::{StopReason, StreamFrame};

/// The stop frame's wire shape, as the bus and the GUI carry it.
#[test]
fn the_stop_frame_serializes_as_its_golden_sample() {
    let frame = StreamFrame::HostStopping {
        reason: StopReason::DataWiped,
    };
    assert_eq!(
        serde_json::to_value(&frame).expect("json"),
        serde_json::json!({ "hostStopping": { "reason": "dataWiped" } })
    );
}
