use rekindle_transport_ipc::v3::conditions::evaluator::{EvalContext, ConditionError};
use rekindle_transport_ipc::v3::conditions::predicates::*;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;

fn base_ctx() -> EvalContext<'static> {
    static PEER: [u8; 32] = [0xAA; 32];
    static TOPIC: [u8; 32] = [0xBB; 32];
    EvalContext {
        sender_peer_id: &PEER,
        sender_clearance: Clearance::Internal,
        frame_body_len: 1024,
        topic_hash: Some(&TOPIC),
        transfer_id: None,
        chunk_index: Some(42),
        wall_clock_secs_since_midnight: 43200,
        event_priority: Some(5),
    }
}

// ── peer_id_equals ───────────────────────────────────────────────

#[test]
fn peer_id_equals_match() {
    assert_eq!(dispatch(PEER_ID_EQUALS, &[0xAA; 32], &base_ctx()), Ok(true));
}

#[test]
fn peer_id_equals_no_match() {
    assert_eq!(dispatch(PEER_ID_EQUALS, &[0xCC; 32], &base_ctx()), Ok(false));
}

#[test]
fn peer_id_equals_wrong_len() {
    assert!(matches!(
        dispatch(PEER_ID_EQUALS, &[0xAA; 16], &base_ctx()),
        Err(ConditionError::MalformedParams { .. })
    ));
}

// ── clearance_min ────────────────────────────────────────────────

#[test]
fn clearance_min_passes_when_above() {
    assert_eq!(dispatch(CLEARANCE_MIN, &[Clearance::Public as u8], &base_ctx()), Ok(true));
}

#[test]
fn clearance_min_passes_when_equal() {
    assert_eq!(dispatch(CLEARANCE_MIN, &[Clearance::Internal as u8], &base_ctx()), Ok(true));
}

#[test]
fn clearance_min_fails_when_below() {
    assert_eq!(dispatch(CLEARANCE_MIN, &[Clearance::Confidential as u8], &base_ctx()), Ok(false));
}

#[test]
fn clearance_min_invalid_tier() {
    assert!(matches!(
        dispatch(CLEARANCE_MIN, &[0xFF], &base_ctx()),
        Err(ConditionError::MalformedParams { .. })
    ));
}

// ── clearance_max ────────────────────────────────────────────────

#[test]
fn clearance_max_passes_when_below() {
    assert_eq!(dispatch(CLEARANCE_MAX, &[Clearance::Confidential as u8], &base_ctx()), Ok(true));
}

#[test]
fn clearance_max_passes_when_equal() {
    assert_eq!(dispatch(CLEARANCE_MAX, &[Clearance::Internal as u8], &base_ctx()), Ok(true));
}

#[test]
fn clearance_max_fails_when_above() {
    assert_eq!(dispatch(CLEARANCE_MAX, &[Clearance::Public as u8], &base_ctx()), Ok(false));
}

// ── topic_equals ─────────────────────────────────────────────────

#[test]
fn topic_equals_match() {
    assert_eq!(dispatch(TOPIC_EQUALS, &[0xBB; 32], &base_ctx()), Ok(true));
}

#[test]
fn topic_equals_no_match() {
    assert_eq!(dispatch(TOPIC_EQUALS, &[0xCC; 32], &base_ctx()), Ok(false));
}

#[test]
fn topic_equals_no_topic_in_context() {
    static PEER: [u8; 32] = [0xAA; 32];
    let ctx = EvalContext {
        sender_peer_id: &PEER,
        sender_clearance: Clearance::Internal,
        frame_body_len: 1024,
        topic_hash: None,
        transfer_id: None,
        chunk_index: None,
        wall_clock_secs_since_midnight: 0,
        event_priority: None,
    };
    assert_eq!(dispatch(TOPIC_EQUALS, &[0xBB; 32], &ctx), Ok(false));
}

// ── time_of_day_within ───────────────────────────────────────────

#[test]
fn time_within_range() {
    let mut params = Vec::new();
    params.extend_from_slice(&40000u32.to_le_bytes());
    params.extend_from_slice(&50000u32.to_le_bytes());
    assert_eq!(dispatch(TIME_OF_DAY_WITHIN, &params, &base_ctx()), Ok(true));
}

#[test]
fn time_outside_range() {
    let mut params = Vec::new();
    params.extend_from_slice(&44000u32.to_le_bytes());
    params.extend_from_slice(&50000u32.to_le_bytes());
    assert_eq!(dispatch(TIME_OF_DAY_WITHIN, &params, &base_ctx()), Ok(false));
}

#[test]
fn time_wraps_midnight() {
    let mut params = Vec::new();
    params.extend_from_slice(&79200u32.to_le_bytes());
    params.extend_from_slice(&21600u32.to_le_bytes());

    static PEER: [u8; 32] = [0xAA; 32];
    let ctx_late = EvalContext {
        sender_peer_id: &PEER,
        sender_clearance: Clearance::Internal,
        frame_body_len: 0,
        topic_hash: None,
        transfer_id: None,
        chunk_index: None,
        wall_clock_secs_since_midnight: 80000,
        event_priority: None,
    };
    assert_eq!(dispatch(TIME_OF_DAY_WITHIN, &params, &ctx_late), Ok(true));

    let ctx_early = EvalContext {
        wall_clock_secs_since_midnight: 10000,
        ..ctx_late
    };
    assert_eq!(dispatch(TIME_OF_DAY_WITHIN, &params, &ctx_early), Ok(true));

    let ctx_afternoon = EvalContext {
        wall_clock_secs_since_midnight: 43200,
        ..ctx_late
    };
    assert_eq!(dispatch(TIME_OF_DAY_WITHIN, &params, &ctx_afternoon), Ok(false));
}

// ── frame_size_max ───────────────────────────────────────────────

#[test]
fn frame_size_within_max() {
    assert_eq!(dispatch(FRAME_SIZE_MAX, &2048u32.to_le_bytes(), &base_ctx()), Ok(true));
}

#[test]
fn frame_size_exactly_max() {
    assert_eq!(dispatch(FRAME_SIZE_MAX, &1024u32.to_le_bytes(), &base_ctx()), Ok(true));
}

#[test]
fn frame_size_exceeds_max() {
    assert_eq!(dispatch(FRAME_SIZE_MAX, &512u32.to_le_bytes(), &base_ctx()), Ok(false));
}

// ── event_priority_min ───────────────────────────────────────────

#[test]
fn event_priority_passes_when_above_min() {
    // ctx has priority 5, min is 3
    assert_eq!(dispatch(EVENT_PRIORITY_MIN, &[3], &base_ctx()), Ok(true));
}

#[test]
fn event_priority_passes_when_equal_to_min() {
    // ctx has priority 5, min is 5
    assert_eq!(dispatch(EVENT_PRIORITY_MIN, &[5], &base_ctx()), Ok(true));
}

#[test]
fn event_priority_fails_when_below_min() {
    // ctx has priority 5, min is 8
    assert_eq!(dispatch(EVENT_PRIORITY_MIN, &[8], &base_ctx()), Ok(false));
}

#[test]
fn event_priority_fails_when_no_priority_in_ctx() {
    static PEER: [u8; 32] = [0xAA; 32];
    let ctx = EvalContext {
        sender_peer_id: &PEER,
        sender_clearance: Clearance::Internal,
        frame_body_len: 0,
        topic_hash: None,
        transfer_id: None,
        chunk_index: None,
        wall_clock_secs_since_midnight: 0,
        event_priority: None,
    };
    assert_eq!(dispatch(EVENT_PRIORITY_MIN, &[1], &ctx), Ok(false));
}

#[test]
fn event_priority_zero_min_passes_any_priority() {
    assert_eq!(dispatch(EVENT_PRIORITY_MIN, &[0], &base_ctx()), Ok(true));
}

#[test]
fn event_priority_max_min_only_passes_max_priority() {
    // ctx has priority 5, min is 255
    assert_eq!(dispatch(EVENT_PRIORITY_MIN, &[255], &base_ctx()), Ok(false));

    static PEER: [u8; 32] = [0xAA; 32];
    let ctx_max = EvalContext {
        sender_peer_id: &PEER,
        sender_clearance: Clearance::Internal,
        frame_body_len: 0,
        topic_hash: None,
        transfer_id: None,
        chunk_index: None,
        wall_clock_secs_since_midnight: 0,
        event_priority: Some(255),
    };
    assert_eq!(dispatch(EVENT_PRIORITY_MIN, &[255], &ctx_max), Ok(true));
}

#[test]
fn event_priority_wrong_param_len() {
    assert!(matches!(
        dispatch(EVENT_PRIORITY_MIN, &[], &base_ctx()),
        Err(ConditionError::MalformedParams { .. })
    ));
    assert!(matches!(
        dispatch(EVENT_PRIORITY_MIN, &[1, 2], &base_ctx()),
        Err(ConditionError::MalformedParams { .. })
    ));
}

// ── peer_id_in_set ───────────────────────────────────────────────

#[test]
fn peer_id_in_set_found() {
    let mut params = Vec::new();
    params.extend_from_slice(&3u16.to_le_bytes());
    params.extend_from_slice(&[0x11; 32]);
    params.extend_from_slice(&[0xAA; 32]);
    params.extend_from_slice(&[0x33; 32]);
    assert_eq!(dispatch(PEER_ID_IN_SET, &params, &base_ctx()), Ok(true));
}

#[test]
fn peer_id_in_set_not_found() {
    let mut params = Vec::new();
    params.extend_from_slice(&2u16.to_le_bytes());
    params.extend_from_slice(&[0x11; 32]);
    params.extend_from_slice(&[0x22; 32]);
    assert_eq!(dispatch(PEER_ID_IN_SET, &params, &base_ctx()), Ok(false));
}

#[test]
fn peer_id_in_set_empty_set() {
    let params = 0u16.to_le_bytes();
    assert_eq!(dispatch(PEER_ID_IN_SET, &params, &base_ctx()), Ok(false));
}

#[test]
fn peer_id_in_set_wrong_param_len() {
    let mut params = Vec::new();
    params.extend_from_slice(&2u16.to_le_bytes());
    params.extend_from_slice(&[0x11; 32]);
    assert!(matches!(
        dispatch(PEER_ID_IN_SET, &params, &base_ctx()),
        Err(ConditionError::MalformedParams { .. })
    ));
}

// ── transfer_id_equals ───────────────────────────────────────────

#[test]
fn transfer_id_equals_no_transfer_in_ctx() {
    assert_eq!(dispatch(TRANSFER_ID_EQUALS, &[0; 16], &base_ctx()), Ok(false));
}

#[test]
fn transfer_id_equals_match() {
    let id = uuid::Uuid::nil();
    static PEER: [u8; 32] = [0xAA; 32];
    let ctx = EvalContext {
        sender_peer_id: &PEER,
        sender_clearance: Clearance::Internal,
        frame_body_len: 0,
        topic_hash: None,
        transfer_id: Some(&id),
        chunk_index: None,
        wall_clock_secs_since_midnight: 0,
        event_priority: None,
    };
    assert_eq!(dispatch(TRANSFER_ID_EQUALS, id.as_bytes(), &ctx), Ok(true));
}

#[test]
fn transfer_id_equals_no_match() {
    let id = uuid::Uuid::nil();
    static PEER: [u8; 32] = [0xAA; 32];
    let ctx = EvalContext {
        sender_peer_id: &PEER,
        sender_clearance: Clearance::Internal,
        frame_body_len: 0,
        topic_hash: None,
        transfer_id: Some(&id),
        chunk_index: None,
        wall_clock_secs_since_midnight: 0,
        event_priority: None,
    };
    assert_eq!(dispatch(TRANSFER_ID_EQUALS, &[0xFF; 16], &ctx), Ok(false));
}

// ── chunk_index_within ───────────────────────────────────────────

#[test]
fn chunk_index_within_range() {
    let mut params = Vec::new();
    params.extend_from_slice(&10u32.to_le_bytes());
    params.extend_from_slice(&50u32.to_le_bytes());
    assert_eq!(dispatch(CHUNK_INDEX_WITHIN, &params, &base_ctx()), Ok(true));
}

#[test]
fn chunk_index_outside_range() {
    let mut params = Vec::new();
    params.extend_from_slice(&0u32.to_le_bytes());
    params.extend_from_slice(&10u32.to_le_bytes());
    assert_eq!(dispatch(CHUNK_INDEX_WITHIN, &params, &base_ctx()), Ok(false));
}

#[test]
fn chunk_index_no_chunk_in_ctx() {
    static PEER: [u8; 32] = [0xAA; 32];
    let ctx = EvalContext {
        sender_peer_id: &PEER,
        sender_clearance: Clearance::Internal,
        frame_body_len: 0,
        topic_hash: None,
        transfer_id: None,
        chunk_index: None,
        wall_clock_secs_since_midnight: 0,
        event_priority: None,
    };
    let mut params = Vec::new();
    params.extend_from_slice(&0u32.to_le_bytes());
    params.extend_from_slice(&100u32.to_le_bytes());
    assert_eq!(dispatch(CHUNK_INDEX_WITHIN, &params, &ctx), Ok(false));
}

// ── unknown predicate ────────────────────────────────────────────

#[test]
fn unknown_predicate_id_returns_error() {
    assert_eq!(
        dispatch(0xBEEF, &[], &base_ctx()),
        Err(ConditionError::UnknownPredicate(0xBEEF))
    );
}
