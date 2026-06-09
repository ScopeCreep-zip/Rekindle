use rekindle_transport_ipc::v3::wire::{
    lane::Lane,
    frame_class::FrameClass,
};

/// Returns true if the given FrameClass is permitted on the given Lane.
/// This encodes the coupling table from the spec; if the spec changes,
/// this function and its tests change together.
fn is_permitted(lane: Lane, class: FrameClass) -> bool {
    matches!(
        (lane, class),
        (Lane::Control, FrameClass::Channel)
        | (Lane::Control, FrameClass::Datagram)
        | (Lane::Data, FrameClass::Stream)
        | (Lane::Audit, FrameClass::Audit)
        | (Lane::Handoff, FrameClass::Handoff)
    )
}

#[test]
fn channel_only_on_control() {
    assert!(is_permitted(Lane::Control, FrameClass::Channel));
    assert!(!is_permitted(Lane::Data, FrameClass::Channel));
    assert!(!is_permitted(Lane::Audit, FrameClass::Channel));
    assert!(!is_permitted(Lane::Handoff, FrameClass::Channel));
}

#[test]
fn datagram_only_on_control() {
    assert!(is_permitted(Lane::Control, FrameClass::Datagram));
    assert!(!is_permitted(Lane::Data, FrameClass::Datagram));
    assert!(!is_permitted(Lane::Audit, FrameClass::Datagram));
    assert!(!is_permitted(Lane::Handoff, FrameClass::Datagram));
}

#[test]
fn stream_only_on_data() {
    assert!(is_permitted(Lane::Data, FrameClass::Stream));
    assert!(!is_permitted(Lane::Control, FrameClass::Stream));
    assert!(!is_permitted(Lane::Audit, FrameClass::Stream));
    assert!(!is_permitted(Lane::Handoff, FrameClass::Stream));
}

#[test]
fn audit_only_on_audit() {
    assert!(is_permitted(Lane::Audit, FrameClass::Audit));
    assert!(!is_permitted(Lane::Control, FrameClass::Audit));
    assert!(!is_permitted(Lane::Data, FrameClass::Audit));
    assert!(!is_permitted(Lane::Handoff, FrameClass::Audit));
}

#[test]
fn handoff_only_on_handoff() {
    assert!(is_permitted(Lane::Handoff, FrameClass::Handoff));
    assert!(!is_permitted(Lane::Control, FrameClass::Handoff));
    assert!(!is_permitted(Lane::Data, FrameClass::Handoff));
    assert!(!is_permitted(Lane::Audit, FrameClass::Handoff));
}

#[test]
fn every_lane_class_pair_is_covered() {
    // Exhaustive: 4 lanes × 5 classes = 20 pairs.
    // Exactly 5 are permitted.
    let mut permitted_count = 0;
    for &lane in &Lane::ALL {
        for &class in &FrameClass::ALL {
            if is_permitted(lane, class) {
                permitted_count += 1;
            }
        }
    }
    assert_eq!(
        permitted_count, 5,
        "Exactly 5 (lane, class) pairs should be permitted"
    );
}
