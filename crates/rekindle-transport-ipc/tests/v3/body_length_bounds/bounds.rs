use rekindle_transport_ipc::v3::wire::constants::*;

// ── Control Lane ──────────────────────────────────────────────────

#[test]
fn control_min_is_18() {
    // 2-byte plaintext (FrameClass + FrameKind) + 16-byte AEAD tag
    assert_eq!(MIN_BODY_LEN_CONTROL, 18);
}

#[test]
fn control_max_is_64_kib() {
    assert_eq!(MAX_BODY_LEN_CONTROL, 64 * 1024);
}

#[test]
fn control_min_less_than_max() {
    assert!(MIN_BODY_LEN_CONTROL < MAX_BODY_LEN_CONTROL);
}

// ── Data Lane ─────────────────────────────────────────────────────

#[test]
fn data_min_is_50() {
    // 32-byte header + 2-byte plaintext + 16-byte AEAD tag
    assert_eq!(MIN_BODY_LEN_DATA, 50);
}

#[test]
fn data_min_accounts_for_header_plus_tag() {
    let expected = STREAM_HEADER_LEN as u32 + 2 + AEAD_TAG_LEN as u32;
    assert_eq!(MIN_BODY_LEN_DATA, expected);
}

#[test]
fn data_max_is_16_mib() {
    assert_eq!(MAX_BODY_LEN_DATA, 16 * 1024 * 1024);
}

#[test]
fn data_min_less_than_max() {
    assert!(MIN_BODY_LEN_DATA < MAX_BODY_LEN_DATA);
}

// ── Audit Lane ────────────────────────────────────────────────────

#[test]
fn audit_min_is_18() {
    assert_eq!(MIN_BODY_LEN_AUDIT, 18);
}

#[test]
fn audit_max_is_1_mib() {
    assert_eq!(MAX_BODY_LEN_AUDIT, 1024 * 1024);
}

#[test]
fn audit_min_less_than_max() {
    assert!(MIN_BODY_LEN_AUDIT < MAX_BODY_LEN_AUDIT);
}

// ── Handoff Lane ──────────────────────────────────────────────────

#[test]
fn handoff_min_is_18() {
    assert_eq!(MIN_BODY_LEN_HANDOFF, 18);
}

#[test]
fn handoff_max_is_4_kib() {
    assert_eq!(MAX_BODY_LEN_HANDOFF, 4 * 1024);
}

#[test]
fn handoff_min_less_than_max() {
    assert!(MIN_BODY_LEN_HANDOFF < MAX_BODY_LEN_HANDOFF);
}

// ── Cross-lane consistency ────────────────────────────────────────

#[test]
fn data_lane_has_largest_max() {
    assert!(MAX_BODY_LEN_DATA > MAX_BODY_LEN_CONTROL);
    assert!(MAX_BODY_LEN_DATA > MAX_BODY_LEN_AUDIT);
    assert!(MAX_BODY_LEN_DATA > MAX_BODY_LEN_HANDOFF);
}

#[test]
fn handoff_lane_has_smallest_max() {
    assert!(MAX_BODY_LEN_HANDOFF < MAX_BODY_LEN_CONTROL);
    assert!(MAX_BODY_LEN_HANDOFF < MAX_BODY_LEN_AUDIT);
    assert!(MAX_BODY_LEN_HANDOFF < MAX_BODY_LEN_DATA);
}

#[test]
fn all_maxes_fit_in_u32() {
    // Envelope body_len is u32. Verify no max exceeds u32::MAX.
    assert!(MAX_BODY_LEN_DATA <= u32::MAX);
}
