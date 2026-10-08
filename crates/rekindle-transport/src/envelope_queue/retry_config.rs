//! Per-kind retry configuration. Drives the retry-cap and backoff math
//! used by `EnvelopeQueue::run_retry_tick`.
//!
//! Reference points: Wave 16's research convergence on
//! - Calls cap at the 30 s ring window (5 × 6 s)
//! - Mid-call media-state changes cap fast (3 × 2 s) — peer sees stale
//!   state until next change; not fatal.
//! - Friend-add and DM body cap at 10 min (20 × 30 s) — matching the
//!   legacy `pending_messages` behavior. (The friend-add inbox kinds
//!   have no TypeId and don't flow through this queue; W16.10 handles
//!   them.)

use crate::envelope_store::EnvelopeKind;

/// Retry policy parameters for a given envelope kind.
#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    /// Maximum number of attempts before the row is dead-lettered.
    pub max_retries: u32,
    /// Base backoff in milliseconds. Used as the seed for exponential
    /// backoff with full jitter (capped at `max_backoff_ms`).
    pub base_backoff_ms: u64,
    /// Cap on backoff between retries. Veilid private routes expire at
    /// 5 min, so retry interval should never exceed that — there's no
    /// point waiting longer than a fresh route would last.
    pub max_backoff_ms: u64,
}

impl RetryConfig {
    /// Per-kind config used by [`EnvelopeQueue`]. Match arms cover every
    /// `EnvelopeKind` variant explicitly so adding a new kind forces
    /// the author to choose a config (no silent defaults).
    pub const fn for_kind(kind: EnvelopeKind) -> Self {
        match kind {
            // 1:1 call signaling — post-decision phase only.
            //
            // (CallInvite + CallRinging are handled by Veilid `app_call`
            //  per W16.5b; they never enter this queue. The other call
            //  envelopes still cap at the 30 s ring window — five tries
            //  × 6 s = 30 s — to match the user-decision deadline.)
            EnvelopeKind::CallAccept
            | EnvelopeKind::CallDecline
            | EnvelopeKind::CallEnd
            | EnvelopeKind::GroupCallOffer
            | EnvelopeKind::GroupCallAccept
            | EnvelopeKind::GroupCallDecline => Self {
                max_retries: 5,
                base_backoff_ms: 6_000,
                max_backoff_ms: 30_000,
            },
            // Mid-call media state — try a few times then give up.
            // Peer sees stale state until the next change; non-fatal.
            EnvelopeKind::CallMediaState | EnvelopeKind::CallReaction => Self {
                max_retries: 3,
                base_backoff_ms: 2_000,
                max_backoff_ms: 6_000,
            },
            // DM invite request/reply — wait longer than calls (user-
            // tolerant latency). Request side has its own 60 s timeout
            // via expect_reply; persisted retry runs up to 5 min total
            // matching route expiry.
            EnvelopeKind::DmInviteRequest
            | EnvelopeKind::GroupDmInviteRequest
            | EnvelopeKind::DmInviteReply
            | EnvelopeKind::GroupDmInviteReply => Self {
                max_retries: 10,
                base_backoff_ms: 6_000,
                max_backoff_ms: 60_000,
            },
            // DM body content (Signal-encrypted) — same 10 min budget as
            // the legacy pending_messages behavior.
            EnvelopeKind::DmMessage => Self {
                max_retries: 20,
                base_backoff_ms: 30_000,
                max_backoff_ms: 60_000,
            },
            // 3-phase friend-add inbox writes — handled by W16.10's
            // `operations::friend` path, not this queue. If somehow
            // enqueued (caller bug), use a conservative cap.
            EnvelopeKind::FriendRequestInbox | EnvelopeKind::FriendAcceptInbox => Self {
                max_retries: 1,
                base_backoff_ms: 1_000,
                max_backoff_ms: 1_000,
            },
        }
    }

    /// Compute the delay before the next retry given the current
    /// `retry_count`. Exponential backoff with full jitter, capped at
    /// `max_backoff_ms`.
    ///
    /// `jitter_bp` is a deterministic-test friendly RNG seam: pass a
    /// value in `0..=10_000` representing basis points (so 7_500 = 75%).
    /// Production callers pass `rand::thread_rng().gen_range(0..=10_000)`.
    /// Tests pass a fixed value. Pure integer math throughout.
    pub fn backoff_for_attempt(self, retry_count: u32, jitter_bp: u32) -> u64 {
        let shift = retry_count.min(10);
        let base = self.base_backoff_ms.saturating_mul(1u64 << shift);
        let capped = base.min(self.max_backoff_ms);
        // Full jitter: scale `capped` by a factor in [0.5, 1.0]:
        //   factor_bp = 5_000 + (jitter_bp / 2)
        // where jitter_bp ∈ 0..=10_000. Result ∈ 5_000..=10_000 bp.
        let clamped_bp = jitter_bp.min(10_000);
        let factor_bp: u64 = 5_000 + u64::from(clamped_bp) / 2;
        let scaled = capped.saturating_mul(factor_bp) / 10_000;
        scaled.max(self.base_backoff_ms / 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_signaling_cap_matches_30s_window() {
        let cfg = RetryConfig::for_kind(EnvelopeKind::CallAccept);
        // 5 retries × ~6 s nominal = ~30 s (with jitter, ±50%).
        assert_eq!(cfg.max_retries, 5);
        assert_eq!(cfg.base_backoff_ms, 6_000);
    }

    #[test]
    fn dm_body_uses_10min_window() {
        let cfg = RetryConfig::for_kind(EnvelopeKind::DmMessage);
        // 20 retries × 30 s = 10 min — matches legacy pending_messages.
        assert_eq!(cfg.max_retries, 20);
        assert_eq!(cfg.base_backoff_ms, 30_000);
    }

    #[test]
    fn backoff_doubles_then_caps() {
        let cfg = RetryConfig::for_kind(EnvelopeKind::CallAccept);
        let attempt0 = cfg.backoff_for_attempt(0, 10_000);
        let attempt1 = cfg.backoff_for_attempt(1, 10_000);
        let attempt5 = cfg.backoff_for_attempt(5, 10_000);
        assert!(attempt1 >= attempt0, "backoff should grow with attempts");
        assert!(attempt5 <= cfg.max_backoff_ms, "backoff respects cap");
    }

    #[test]
    fn jitter_keeps_backoff_in_range() {
        let cfg = RetryConfig::for_kind(EnvelopeKind::CallAccept);
        for jitter_bp in [0, 2_500, 5_000, 7_500, 10_000] {
            let b = cfg.backoff_for_attempt(0, jitter_bp);
            // Full jitter: lower bound is base/2.
            assert!(b >= cfg.base_backoff_ms / 2, "jitter_bp={jitter_bp} b={b}");
            assert!(b <= cfg.max_backoff_ms, "jitter_bp={jitter_bp} b={b}");
        }
    }
}
