//! Built-in predicate implementations.
//!
//! Each predicate takes its params slice and the EvalContext,
//! returns Ok(true) if the condition is satisfied, Ok(false) if not,
//! Err if params are malformed.

use crate::v3::conditions::evaluator::{ConditionError, EvalContext};
use crate::v3::wire::clearance::Clearance;

pub const PEER_ID_EQUALS: u16 = 0x0001;
pub const CLEARANCE_MIN: u16 = 0x0002;
pub const CLEARANCE_MAX: u16 = 0x0003;
pub const TOPIC_EQUALS: u16 = 0x0004;
pub const TIME_OF_DAY_WITHIN: u16 = 0x0005;
pub const FRAME_SIZE_MAX: u16 = 0x0006;
pub const EVENT_PRIORITY_MIN: u16 = 0x0007;
pub const PEER_ID_IN_SET: u16 = 0x0008;
pub const TRANSFER_ID_EQUALS: u16 = 0x0009;
pub const CHUNK_INDEX_WITHIN: u16 = 0x000A;

/// Dispatch a predicate by its ID.
pub fn dispatch(predicate_id: u16, params: &[u8], ctx: &EvalContext<'_>) -> Result<bool, ConditionError> {
    match predicate_id {
        PEER_ID_EQUALS => peer_id_equals(params, ctx),
        CLEARANCE_MIN => clearance_min(params, ctx),
        CLEARANCE_MAX => clearance_max(params, ctx),
        TOPIC_EQUALS => topic_equals(params, ctx),
        TIME_OF_DAY_WITHIN => time_of_day_within(params, ctx),
        FRAME_SIZE_MAX => frame_size_max(params, ctx),
        EVENT_PRIORITY_MIN => event_priority_min(params, ctx),
        PEER_ID_IN_SET => peer_id_in_set(params, ctx),
        TRANSFER_ID_EQUALS => transfer_id_equals(params, ctx),
        CHUNK_INDEX_WITHIN => chunk_index_within(params, ctx),
        unknown => Err(ConditionError::UnknownPredicate(unknown)),
    }
}

fn peer_id_equals(params: &[u8], ctx: &EvalContext<'_>) -> Result<bool, ConditionError> {
    if params.len() != 32 {
        return Err(ConditionError::MalformedParams {
            predicate_id: PEER_ID_EQUALS,
            detail: "expected 32-byte peer_id",
        });
    }
    let expected: &[u8; 32] = params.try_into().expect("len checked");
    Ok(ctx.sender_peer_id == expected)
}

fn clearance_min(params: &[u8], ctx: &EvalContext<'_>) -> Result<bool, ConditionError> {
    if params.len() != 1 {
        return Err(ConditionError::MalformedParams {
            predicate_id: CLEARANCE_MIN,
            detail: "expected 1-byte clearance tier",
        });
    }
    let min_tier = Clearance::try_from(params[0]).map_err(|_| ConditionError::MalformedParams {
        predicate_id: CLEARANCE_MIN,
        detail: "invalid clearance tier",
    })?;
    Ok(ctx.sender_clearance >= min_tier)
}

fn clearance_max(params: &[u8], ctx: &EvalContext<'_>) -> Result<bool, ConditionError> {
    if params.len() != 1 {
        return Err(ConditionError::MalformedParams {
            predicate_id: CLEARANCE_MAX,
            detail: "expected 1-byte clearance tier",
        });
    }
    let max_tier = Clearance::try_from(params[0]).map_err(|_| ConditionError::MalformedParams {
        predicate_id: CLEARANCE_MAX,
        detail: "invalid clearance tier",
    })?;
    Ok(ctx.sender_clearance <= max_tier)
}

fn topic_equals(params: &[u8], ctx: &EvalContext<'_>) -> Result<bool, ConditionError> {
    if params.len() != 32 {
        return Err(ConditionError::MalformedParams {
            predicate_id: TOPIC_EQUALS,
            detail: "expected 32-byte topic_hash",
        });
    }
    let expected: &[u8; 32] = params.try_into().expect("len checked");
    match ctx.topic_hash {
        Some(actual) => Ok(actual == expected),
        None => Ok(false),
    }
}

fn time_of_day_within(params: &[u8], ctx: &EvalContext<'_>) -> Result<bool, ConditionError> {
    if params.len() != 8 {
        return Err(ConditionError::MalformedParams {
            predicate_id: TIME_OF_DAY_WITHIN,
            detail: "expected 8 bytes (start_secs u32 LE + end_secs u32 LE)",
        });
    }
    let start = u32::from_le_bytes([params[0], params[1], params[2], params[3]]);
    let end = u32::from_le_bytes([params[4], params[5], params[6], params[7]]);
    let now = ctx.wall_clock_secs_since_midnight;
    if start <= end {
        Ok(now >= start && now <= end)
    } else {
        // Wraps midnight: e.g., 22:00 to 06:00
        Ok(now >= start || now <= end)
    }
}

fn frame_size_max(params: &[u8], ctx: &EvalContext<'_>) -> Result<bool, ConditionError> {
    if params.len() != 4 {
        return Err(ConditionError::MalformedParams {
            predicate_id: FRAME_SIZE_MAX,
            detail: "expected 4-byte u32 LE max",
        });
    }
    let max = u32::from_le_bytes([params[0], params[1], params[2], params[3]]);
    Ok(ctx.frame_body_len <= max)
}

fn event_priority_min(params: &[u8], ctx: &EvalContext<'_>) -> Result<bool, ConditionError> {
    if params.len() != 1 {
        return Err(ConditionError::MalformedParams {
            predicate_id: EVENT_PRIORITY_MIN,
            detail: "expected 1-byte minimum priority",
        });
    }
    let min_priority = params[0];
    match ctx.event_priority {
        Some(priority) => Ok(priority >= min_priority),
        None => Ok(false),
    }
}

fn peer_id_in_set(params: &[u8], ctx: &EvalContext<'_>) -> Result<bool, ConditionError> {
    if params.len() < 2 {
        return Err(ConditionError::MalformedParams {
            predicate_id: PEER_ID_IN_SET,
            detail: "expected at least 2-byte count",
        });
    }
    let count = u16::from_le_bytes([params[0], params[1]]) as usize;
    let expected_len = 2 + count * 32;
    if params.len() != expected_len {
        return Err(ConditionError::MalformedParams {
            predicate_id: PEER_ID_IN_SET,
            detail: "param length does not match count * 32",
        });
    }
    for i in 0..count {
        let start = 2 + i * 32;
        let candidate: &[u8; 32] = params[start..start + 32].try_into().expect("len checked");
        if ctx.sender_peer_id == candidate {
            return Ok(true);
        }
    }
    Ok(false)
}

fn transfer_id_equals(params: &[u8], ctx: &EvalContext<'_>) -> Result<bool, ConditionError> {
    if params.len() != 16 {
        return Err(ConditionError::MalformedParams {
            predicate_id: TRANSFER_ID_EQUALS,
            detail: "expected 16-byte UUID",
        });
    }
    match ctx.transfer_id {
        Some(actual) => Ok(actual.as_bytes() == params),
        None => Ok(false),
    }
}

fn chunk_index_within(params: &[u8], ctx: &EvalContext<'_>) -> Result<bool, ConditionError> {
    if params.len() != 8 {
        return Err(ConditionError::MalformedParams {
            predicate_id: CHUNK_INDEX_WITHIN,
            detail: "expected 8 bytes (start u32 LE + end u32 LE)",
        });
    }
    let start = u32::from_le_bytes([params[0], params[1], params[2], params[3]]);
    let end = u32::from_le_bytes([params[4], params[5], params[6], params[7]]);
    match ctx.chunk_index {
        Some(idx) => Ok(idx >= start && idx <= end),
        None => Ok(false),
    }
}
