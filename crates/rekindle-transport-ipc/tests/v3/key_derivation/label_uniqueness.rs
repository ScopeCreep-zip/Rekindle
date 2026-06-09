use std::collections::HashSet;
use rekindle_transport_ipc::v3::wire::constants::*;

#[test]
fn all_hkdf_labels_are_unique() {
    let labels = [
        LABEL_ENVELOPE_D2L,
        LABEL_ENVELOPE_L2D,
        LABEL_HEADER_D2L,
        LABEL_HEADER_L2D,
        LABEL_STREAM_D2L,
        LABEL_STREAM_L2D,
        LABEL_AUDIT_D2L,
        LABEL_AUDIT_L2D,
        LABEL_HANDOFF,
    ];
    let mut seen = HashSet::new();
    for label in labels {
        assert!(
            seen.insert(label),
            "Duplicate HKDF label: {label:?}. \
             Key reuse across domains is catastrophic."
        );
    }
}

#[test]
fn label_count_is_nine() {
    // 4 key types × 2 directions + 1 symmetric = 9
    let labels = [
        LABEL_ENVELOPE_D2L, LABEL_ENVELOPE_L2D,
        LABEL_HEADER_D2L, LABEL_HEADER_L2D,
        LABEL_STREAM_D2L, LABEL_STREAM_L2D,
        LABEL_AUDIT_D2L, LABEL_AUDIT_L2D,
        LABEL_HANDOFF,
    ];
    assert_eq!(labels.len(), 9);
}
