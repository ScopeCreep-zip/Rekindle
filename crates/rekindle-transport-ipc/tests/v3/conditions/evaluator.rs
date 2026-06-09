use rekindle_transport_ipc::v3::conditions::evaluator::{
    evaluate, encode_condition, EvalContext, ConditionError,
};
use rekindle_transport_ipc::v3::conditions::predicates;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;

fn base_ctx() -> EvalContext<'static> {
    static PEER: [u8; 32] = [0xAA; 32];
    EvalContext {
        sender_peer_id: &PEER,
        sender_clearance: Clearance::Internal,
        frame_body_len: 1024,
        topic_hash: None,
        transfer_id: None,
        chunk_index: None,
        wall_clock_secs_since_midnight: 43200,
        event_priority: Some(5),
    }
}

#[test]
fn empty_conditions_pass() {
    assert_eq!(evaluate(&[], &base_ctx(), 16), Ok(true));
}

#[test]
fn single_passing_condition() {
    let cond = encode_condition(predicates::PEER_ID_EQUALS, &[0xAA; 32]);
    assert_eq!(evaluate(&cond, &base_ctx(), 16), Ok(true));
}

#[test]
fn single_failing_condition() {
    let cond = encode_condition(predicates::PEER_ID_EQUALS, &[0xBB; 32]);
    assert_eq!(evaluate(&cond, &base_ctx(), 16), Ok(false));
}

#[test]
fn two_conditions_both_pass() {
    let mut arr = encode_condition(predicates::PEER_ID_EQUALS, &[0xAA; 32]);
    arr.extend(encode_condition(predicates::FRAME_SIZE_MAX, &2048u32.to_le_bytes()));
    assert_eq!(evaluate(&arr, &base_ctx(), 16), Ok(true));
}

#[test]
fn two_conditions_second_fails() {
    let mut arr = encode_condition(predicates::PEER_ID_EQUALS, &[0xAA; 32]);
    arr.extend(encode_condition(predicates::FRAME_SIZE_MAX, &512u32.to_le_bytes())); // 1024 > 512
    assert_eq!(evaluate(&arr, &base_ctx(), 16), Ok(false));
}

#[test]
fn short_circuits_on_first_false() {
    // First fails, second would fail with unknown predicate — but we never get there
    let mut arr = encode_condition(predicates::PEER_ID_EQUALS, &[0xBB; 32]);
    arr.extend(encode_condition(0xFFFF, &[])); // unknown predicate
    assert_eq!(evaluate(&arr, &base_ctx(), 16), Ok(false));
}

#[test]
fn unknown_predicate_returns_error() {
    let cond = encode_condition(0xFFFF, &[]);
    assert_eq!(
        evaluate(&cond, &base_ctx(), 16),
        Err(ConditionError::UnknownPredicate(0xFFFF))
    );
}

#[test]
fn malformed_params_returns_error() {
    let cond = encode_condition(predicates::PEER_ID_EQUALS, &[0xAA; 16]); // needs 32, got 16
    assert!(matches!(
        evaluate(&cond, &base_ctx(), 16),
        Err(ConditionError::MalformedParams { .. })
    ));
}

#[test]
fn truncated_condition_header_returns_too_short() {
    let buf = [0x01, 0x00, 0x20]; // only 3 bytes, need 4 for header
    assert_eq!(evaluate(&buf, &base_ctx(), 16), Err(ConditionError::TooShort));
}

#[test]
fn truncated_param_body_returns_too_short() {
    // Header says param_len=32 but only 16 bytes follow
    let mut buf = Vec::new();
    buf.extend_from_slice(&predicates::PEER_ID_EQUALS.to_le_bytes());
    buf.extend_from_slice(&32u16.to_le_bytes());
    buf.extend_from_slice(&[0xAA; 16]); // only 16 of 32
    assert_eq!(evaluate(&buf, &base_ctx(), 16), Err(ConditionError::TooShort));
}

#[test]
fn too_many_conditions_rejected() {
    let mut arr = Vec::new();
    for _ in 0..4 {
        arr.extend(encode_condition(predicates::FRAME_SIZE_MAX, &2048u32.to_le_bytes()));
    }
    assert_eq!(
        evaluate(&arr, &base_ctx(), 3),
        Err(ConditionError::TooManyConditions { count: 4, max: 3 })
    );
}

#[test]
fn exactly_max_conditions_accepted() {
    let mut arr = Vec::new();
    for _ in 0..3 {
        arr.extend(encode_condition(predicates::FRAME_SIZE_MAX, &2048u32.to_le_bytes()));
    }
    assert_eq!(evaluate(&arr, &base_ctx(), 3), Ok(true));
}

#[test]
fn encode_condition_roundtrip_via_evaluate() {
    let cond = encode_condition(predicates::CLEARANCE_MIN, &[Clearance::Public as u8]);
    assert_eq!(evaluate(&cond, &base_ctx(), 16), Ok(true)); // Internal >= Public
}

#[test]
fn multiple_conditions_16_all_pass() {
    let mut arr = Vec::new();
    for _ in 0..16 {
        arr.extend(encode_condition(
            predicates::FRAME_SIZE_MAX,
            &u32::MAX.to_le_bytes(),
        ));
    }
    assert_eq!(evaluate(&arr, &base_ctx(), 16), Ok(true));
}
