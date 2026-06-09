//! Conditions array evaluator — iterates predicate records,
//! dispatches each to the predicate registry, short-circuits on False.

use crate::v3::conditions::predicates;
use crate::v3::wire::clearance::Clearance;

/// Facts available about the current frame and session for predicate evaluation.
pub struct EvalContext<'a> {
    pub sender_peer_id: &'a [u8; 32],
    pub sender_clearance: Clearance,
    pub frame_body_len: u32,
    pub topic_hash: Option<&'a [u8; 32]>,
    pub transfer_id: Option<&'a uuid::Uuid>,
    pub chunk_index: Option<u32>,
    pub wall_clock_secs_since_midnight: u32,
    pub event_priority: Option<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConditionError {
    UnknownPredicate(u16),
    MalformedParams { predicate_id: u16, detail: &'static str },
    TooShort,
    TooManyConditions { count: usize, max: usize },
}

/// Evaluate a serialized conditions array against the given context.
///
/// The array is a concatenation of records:
///   `[u16 LE predicate_id][u16 LE param_len][param_len bytes params]`
///
/// Returns `Ok(true)` if all conditions pass (or the array is empty),
/// `Ok(false)` if any condition evaluates false, and `Err` on malformed input.
pub fn evaluate(conditions: &[u8], ctx: &EvalContext<'_>, max_conditions: usize) -> Result<bool, ConditionError> {
    if conditions.is_empty() {
        return Ok(true);
    }

    let mut offset = 0;
    let mut count = 0;

    while offset < conditions.len() {
        count += 1;
        if count > max_conditions {
            return Err(ConditionError::TooManyConditions { count, max: max_conditions });
        }

        if offset + 4 > conditions.len() {
            return Err(ConditionError::TooShort);
        }

        let predicate_id = u16::from_le_bytes([conditions[offset], conditions[offset + 1]]);
        let param_len = u16::from_le_bytes([conditions[offset + 2], conditions[offset + 3]]) as usize;
        offset += 4;

        if offset + param_len > conditions.len() {
            return Err(ConditionError::TooShort);
        }

        let params = &conditions[offset..offset + param_len];
        offset += param_len;

        let result = predicates::dispatch(predicate_id, params, ctx)?;
        if !result {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Build a single condition record for embedding in a conditions array.
pub fn encode_condition(predicate_id: u16, params: &[u8]) -> Vec<u8> {
    let param_len = u16::try_from(params.len()).expect("condition params exceed u16");
    let mut buf = Vec::with_capacity(4 + params.len());
    buf.extend_from_slice(&predicate_id.to_le_bytes());
    buf.extend_from_slice(&param_len.to_le_bytes());
    buf.extend_from_slice(params);
    buf
}
