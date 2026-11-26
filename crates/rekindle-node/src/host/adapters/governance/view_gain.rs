//! Re-reading the channel outboxes when a merge opens a channel to us
//! (plan E3.2 SEC). The channel ingest withholds a post of a channel we
//! may not view (`channel/ingest.rs`, a reader-side gate until E3.5's
//! per-channel keys) and its outbox cursor passes it, so a later grant of
//! `VIEW_CHANNELS` (an overwrite lifted, a role assigned) must read the
//! outboxes again: the same cursor reset an older key's arrival runs
//! (`key_history.rs`, `catchup.rs` `spawn_outbox_reread`, D16). Posts we
//! already hold come back as duplicates and are skipped.

use rekindle_governance::permissions::compute_permissions;
use rekindle_governance::state::GovernanceState;
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::VIEW_CHANNELS;

/// Whether `next` lets us (`me_hex`, judged as the ingest gate judges it)
/// view a channel `prev` closed to us. The first merge (`prev` none) is
/// followed by the load's own catch-up, so it owes no re-read.
#[must_use]
pub(crate) fn view_gained(
    prev: Option<&GovernanceState>,
    next: &GovernanceState,
    me_hex: &str,
    now_secs: u64,
) -> bool {
    let Some(prev) = prev else {
        return false;
    };
    let me = PseudonymKey::from_hex_lossy(me_hex);
    next.channels.keys().any(|channel| {
        let may = |state: &GovernanceState| {
            compute_permissions(&me, Some(channel), state, now_secs) & VIEW_CHANNELS != 0
        };
        may(next) && !may(prev)
    })
}
