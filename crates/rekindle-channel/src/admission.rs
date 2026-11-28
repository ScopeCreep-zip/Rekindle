//! Reader-side authorization at a post's signed governance cut.

use rekindle_governance::permissions::{compute_permissions, has_capability};
use rekindle_governance::state::GovernanceState;
use rekindle_types::dto::channel_message::PostRefusal;
use rekindle_types::id::{ChannelId, PseudonymKey, ThreadId};
use rekindle_types::permissions::{SEND_IN_THREADS, SEND_MESSAGES};

/// Check the author's membership, timeout and stream permissions at a resolved cut.
///
/// # Errors
/// The author or the stream does not have standing at the cut.
pub fn authorize_post(
    state: &GovernanceState,
    author: &PseudonymKey,
    stream: ChannelId,
    thread: bool,
    sent_at_secs: u64,
) -> Result<(), PostRefusal> {
    if !state.members.contains_key(author) || state.is_departed(author) {
        return Err(PostRefusal::NotMember);
    }
    let permission_time = match state.timeouts.get(author) {
        Some(timeout) if sent_at_secs < timeout.started_at => {
            // The permission evaluator checks expiry. Evaluate outside the
            // interval when the signed post predates the timeout's start.
            timeout.started_at.saturating_add(timeout.duration_seconds)
        }
        Some(timeout) if !timeout.is_expired(sent_at_secs) => return Err(PostRefusal::TimedOut),
        _ => sent_at_secs,
    };
    let (channel, needed) = if thread {
        let thread = state
            .threads
            .get(&ThreadId(stream.0))
            .ok_or(PostRefusal::UnknownStream)?;
        (thread.parent_channel_id, SEND_IN_THREADS)
    } else {
        (stream, SEND_MESSAGES)
    };
    if !state.channels.contains_key(&channel) {
        return Err(PostRefusal::UnknownStream);
    }
    let effective = compute_permissions(author, Some(&channel), state, permission_time);
    has_capability(effective, needed)
        .then_some(())
        .ok_or(PostRefusal::MissingPermission)
}
