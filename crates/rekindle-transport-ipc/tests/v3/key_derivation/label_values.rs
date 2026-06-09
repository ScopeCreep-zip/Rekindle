use rekindle_transport_ipc::v3::wire::constants::*;

// These tests pin the exact label strings. A single-character change
// produces a completely different derived key. Both sides must agree
// on the exact string.

#[test]
fn envelope_d2l_label() {
    assert_eq!(LABEL_ENVELOPE_D2L, "rti-envelope-d2l-v1");
}

#[test]
fn envelope_l2d_label() {
    assert_eq!(LABEL_ENVELOPE_L2D, "rti-envelope-l2d-v1");
}

#[test]
fn header_d2l_label() {
    assert_eq!(LABEL_HEADER_D2L, "rti-header-d2l-v1");
}

#[test]
fn header_l2d_label() {
    assert_eq!(LABEL_HEADER_L2D, "rti-header-l2d-v1");
}

#[test]
fn stream_d2l_label() {
    assert_eq!(LABEL_STREAM_D2L, "rti-stream-d2l-v1");
}

#[test]
fn stream_l2d_label() {
    assert_eq!(LABEL_STREAM_L2D, "rti-stream-l2d-v1");
}

#[test]
fn audit_d2l_label() {
    assert_eq!(LABEL_AUDIT_D2L, "rti-audit-d2l-v1");
}

#[test]
fn audit_l2d_label() {
    assert_eq!(LABEL_AUDIT_L2D, "rti-audit-l2d-v1");
}

#[test]
fn handoff_label() {
    assert_eq!(LABEL_HANDOFF, "rti-handoff-v1");
}

// Structural pattern checks — every directional label follows the
// "rti-{purpose}-{direction}-v1" pattern.

#[test]
fn all_directional_labels_follow_naming_convention() {
    let directional = [
        LABEL_ENVELOPE_D2L, LABEL_ENVELOPE_L2D,
        LABEL_HEADER_D2L, LABEL_HEADER_L2D,
        LABEL_STREAM_D2L, LABEL_STREAM_L2D,
        LABEL_AUDIT_D2L, LABEL_AUDIT_L2D,
    ];
    for label in directional {
        assert!(label.starts_with("rti-"), "{label} must start with 'rti-'");
        assert!(label.ends_with("-v1"), "{label} must end with '-v1'");
        assert!(
            label.contains("-d2l-") || label.contains("-l2d-"),
            "{label} must contain '-d2l-' or '-l2d-'"
        );
    }
}

#[test]
fn handoff_label_has_no_direction() {
    assert!(!LABEL_HANDOFF.contains("d2l"));
    assert!(!LABEL_HANDOFF.contains("l2d"));
}
