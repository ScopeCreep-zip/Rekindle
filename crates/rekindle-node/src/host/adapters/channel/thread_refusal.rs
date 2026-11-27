//! A timed-out member's thread is refused at the Host's send path (plan
//! E3.1 F3c). Governance validates `ThreadCreated` at time 0, since a CRDT
//! merge reads no clock, so the timeout is checked here by the clock port;
//! a peer's such thread is held at receipt by the same rule
//! (`rekindle_channel::ingest::hold_timed_out_threads`, from
//! `CommunityStore::project_governance`).

use rekindle_channel::error::ChannelError;
use rekindle_channel::ingest::timed_out_until;
use rekindle_governance::state::GovernanceState;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

/// Refuse `entry` when it creates a thread and `writer` is timed out at
/// `now_secs`.
///
/// # Errors
/// `PermissionDenied`, naming when the timeout ends.
pub(super) fn refuse_timed_out_thread(
    entry: &GovernanceEntry,
    gov: &GovernanceState,
    writer: &PseudonymKey,
    now_secs: u64,
) -> Result<(), ChannelError> {
    if !matches!(entry, GovernanceEntry::ThreadCreated { .. }) {
        return Ok(());
    }
    match timed_out_until(gov, writer, now_secs) {
        Some(until_secs) => Err(ChannelError::PermissionDenied(format!(
            "timed out until {until_secs}: no thread can be created"
        ))),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "thread_refusal_tests.rs"]
mod tests;
