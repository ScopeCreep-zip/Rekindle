use super::*;
use crate::test_mock::MockDeps;

/// Phase F — round-trip `EnvelopeRejected` through the MockDeps
/// channel to confirm the wire shape (community_id, sender_pseudonym,
/// reason) is preserved verbatim AND that the `"<unknown>"` sentinel
/// values survive a Clone / Debug round-trip without being normalised
/// away by a stray default.
#[test]
fn envelope_rejected_preserves_fields_and_sentinels() {
    let deps = MockDeps::new();
    deps.emit_event(VideoFlowEvent::EnvelopeRejected {
        community_id: "community_abc".to_string(),
        sender_pseudonym: "deadbeef".to_string(),
        reason: "bad signature".to_string(),
    });
    // Sentinel-bearing variant.
    deps.emit_event(VideoFlowEvent::EnvelopeRejected {
        community_id: "<unknown>".to_string(),
        sender_pseudonym: "<unknown>".to_string(),
        reason: "deserialize failed".to_string(),
    });

    let calls = deps.calls.lock();
    assert_eq!(calls.events.len(), 2);

    match &calls.events[0] {
        VideoFlowEvent::EnvelopeRejected {
            community_id,
            sender_pseudonym,
            reason,
        } => {
            assert_eq!(community_id, "community_abc");
            assert_eq!(sender_pseudonym, "deadbeef");
            assert_eq!(reason, "bad signature");
        }
        other => panic!("expected EnvelopeRejected, got {other:?}"),
    }
    match &calls.events[1] {
        VideoFlowEvent::EnvelopeRejected {
            community_id,
            sender_pseudonym,
            reason,
        } => {
            assert_eq!(community_id, "<unknown>");
            assert_eq!(sender_pseudonym, "<unknown>");
            assert_eq!(reason, "deserialize failed");
        }
        other => panic!("expected EnvelopeRejected sentinel, got {other:?}"),
    }
}
