//! One info line per call operation entering the engine (finding OBS1): a
//! real-app group-call run logged nothing a failed Cancel could be read
//! from. Each line names the operation, the call and group ids and the
//! outcome; ids are written whole and tagged by the log sink's scrub
//! (`rekindle_utils::log_scrub`), the convention every line in this crate
//! follows. Never key material, never message content, never per packet.

use crate::error::CallError;
use crate::signaling::deps::CallSignalingDeps;
use crate::signaling::outbound::{
    accept_dm_call_untraced, accept_group_call_untraced, decline_dm_call_untraced,
    decline_group_call_untraced, end_dm_call_untraced, end_group_call_untraced,
    start_dm_call_untraced, start_group_call_untraced,
};
use crate::state::CallKind;

/// The outcome words of one call operation: `ok`, `refused: <why>` for a
/// refusal the engine decided (not a member, the group refuses calls), or
/// `error: <why>`.
pub(crate) fn outcome<T>(result: &Result<T, CallError>) -> String {
    match result {
        Ok(_) => "ok".to_owned(),
        Err(e @ (CallError::NotInGroup(_) | CallError::GroupCallRefused(_))) => {
            format!("refused: {e}")
        }
        Err(e) => format!("error: {e}"),
    }
}

/// Log one call operation and its result at info.
pub(crate) fn op<T>(
    op: &'static str,
    call: Option<&str>,
    group: Option<&str>,
    result: &Result<T, CallError>,
) {
    let outcome = outcome(result);
    tracing::info!(
        op,
        call = call.unwrap_or("-"),
        group = group.unwrap_or("-"),
        %outcome,
        "call op"
    );
}

// ── Traced entry points (finding OBS1): one info line per operation ──

fn group_of<D: CallSignalingDeps + ?Sized>(deps: &D, call_id: &str) -> Option<String> {
    deps.group_registry()
        .snapshot(call_id)
        .and_then(|s| s.group_id)
}

/// Start a 1:1 call (`start_dm_call_untraced`); returns its `call_id`.
///
/// # Errors
/// The identity, the peer or the send failed.
pub async fn start_dm_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    peer_public_key: &str,
    kind: CallKind,
) -> Result<String, CallError> {
    let result = start_dm_call_untraced(deps, peer_public_key, kind).await;
    op("start dm", result.as_deref().ok(), None, &result);
    result
}

/// Accept a ringing 1:1 call (`accept_dm_call_untraced`).
///
/// # Errors
/// The call is unknown or not ringing, or the accept failed.
pub async fn accept_dm_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
) -> Result<(), CallError> {
    let result = accept_dm_call_untraced(deps, call_id).await;
    op("accept dm", Some(call_id), None, &result);
    result
}

/// Decline a ringing 1:1 call (`decline_dm_call_untraced`).
///
/// # Errors
/// The call is unknown.
pub async fn decline_dm_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
    reason: Option<String>,
) -> Result<(), CallError> {
    let result = decline_dm_call_untraced(deps, call_id, reason).await;
    op("decline dm", Some(call_id), None, &result);
    result
}

/// End (or cancel while dialing) a 1:1 call (`end_dm_call_untraced`).
///
/// # Errors
/// The call is unknown.
pub async fn end_dm_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
    reason: Option<String>,
) -> Result<(), CallError> {
    let result = end_dm_call_untraced(deps, call_id, reason).await;
    op("end dm", Some(call_id), None, &result);
    result
}

/// Start a group call (`start_group_call_untraced`); returns its `call_id`.
///
/// # Errors
/// [`CallError::GroupCallRefused`], [`CallError::NotInGroup`], or the
/// identity or roster could not be read.
pub async fn start_group_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    participant_pubkeys: Vec<String>,
    kind: CallKind,
    group_id: Option<String>,
) -> Result<String, CallError> {
    let group = group_id.clone();
    let result = start_group_call_untraced(deps, participant_pubkeys, kind, group_id).await;
    op(
        "start group",
        result.as_deref().ok(),
        group.as_deref(),
        &result,
    );
    result
}

/// Accept a ringing group call (`accept_group_call_untraced`).
///
/// # Errors
/// The call is unknown or not ringing.
pub async fn accept_group_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
) -> Result<(), CallError> {
    let group = group_of(deps, call_id);
    let result = accept_group_call_untraced(deps, call_id).await;
    op("accept group", Some(call_id), group.as_deref(), &result);
    result
}

/// Decline a ringing group call (`decline_group_call_untraced`).
///
/// # Errors
/// The call is unknown.
pub async fn decline_group_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
    reason: Option<String>,
) -> Result<(), CallError> {
    let group = group_of(deps, call_id);
    let result = decline_group_call_untraced(deps, call_id, reason).await;
    op("decline group", Some(call_id), group.as_deref(), &result);
    result
}

/// Leave, end or cancel a group call (`end_group_call_untraced`).
///
/// # Errors
/// The call is unknown.
pub async fn end_group_call<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
    reason: Option<String>,
) -> Result<(), CallError> {
    let group = group_of(deps, call_id);
    let result = end_group_call_untraced(deps, call_id, reason).await;
    op("end group", Some(call_id), group.as_deref(), &result);
    result
}

/// Log one incoming group call signal at info (finding GV3: OBS1 traced
/// only our own operations, so an end that never arrived left no line):
/// the signal, the call, its sender and what the engine did with it.
pub(crate) fn incoming(signal: &'static str, call: &str, from: &str, outcome: &str) {
    tracing::info!(signal, call, from, outcome, "call signal in");
}

#[cfg(test)]
#[path = "trace_tests.rs"]
mod tests;
