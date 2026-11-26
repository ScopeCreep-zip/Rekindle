//! Per-connection token buckets (plan E4.1): requests and the `MediaUp`
//! frame lane each have their own, so camera frames never starve a client's
//! requests and requests never its frames.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use super::{MEDIA_UP_MAX_FRAMES_PER_SEC, RATE_LIMIT_MAX_TOKENS, RATE_LIMIT_REFILL_MS};

/// `max` tokens, refilled every [`RATE_LIMIT_REFILL_MS`].
struct TokenBucket {
    tokens: AtomicU32,
    last_refill: AtomicU64,
    max: u32,
}

impl TokenBucket {
    fn new(max: u32) -> Self {
        Self {
            tokens: AtomicU32::new(max),
            last_refill: AtomicU64::new(0),
            max,
        }
    }

    /// Spend one token; `false` when the bucket is empty.
    fn take(&self, now_ms: u64) -> bool {
        if now_ms.saturating_sub(self.last_refill.load(Ordering::Relaxed)) >= RATE_LIMIT_REFILL_MS {
            self.tokens.store(self.max, Ordering::Relaxed);
            self.last_refill.store(now_ms, Ordering::Relaxed);
        }
        self.tokens
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |t| t.checked_sub(1))
            .is_ok()
    }
}

/// What a frame spends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lane {
    /// A daemon `Media` frame: exempt (the buckets bound clients).
    DaemonMedia,
    /// A `MediaUp` frame: the frame bucket only.
    MediaUp,
    /// Everything else: the request bucket.
    Request,
}

/// One connection's buckets.
pub(super) struct RateBuckets {
    requests: TokenBucket,
    media_up: TokenBucket,
    /// `MediaUp` frames dropped past the frame bucket on this connection.
    media_up_dropped: AtomicU64,
}

impl RateBuckets {
    pub(super) fn new() -> Self {
        Self {
            requests: TokenBucket::new(RATE_LIMIT_MAX_TOKENS),
            media_up: TokenBucket::new(MEDIA_UP_MAX_FRAMES_PER_SEC),
            media_up_dropped: AtomicU64::new(0),
        }
    }

    /// Whether a frame on `lane` may pass now; a refused `MediaUp` frame is
    /// counted.
    pub(super) fn admit(&self, lane: Lane, now_ms: u64) -> bool {
        match lane {
            Lane::DaemonMedia => true,
            Lane::Request => self.requests.take(now_ms),
            Lane::MediaUp => {
                let ok = self.media_up.take(now_ms);
                if !ok {
                    self.media_up_dropped.fetch_add(1, Ordering::Relaxed);
                }
                ok
            }
        }
    }

    /// `MediaUp` frames dropped past the frame bucket so far.
    pub(super) fn media_up_dropped(&self) -> u64 {
        self.media_up_dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_bucket_admits_240_a_second_and_never_spends_a_request_token() {
        let buckets = RateBuckets::new();
        let now = 10_000;
        for i in 0..MEDIA_UP_MAX_FRAMES_PER_SEC {
            assert!(buckets.admit(Lane::MediaUp, now), "frame {i} within 240/s");
        }
        assert!(
            !buckets.admit(Lane::MediaUp, now),
            "the 241st frame is dropped"
        );
        assert_eq!(buckets.media_up_dropped(), 1, "and counted");
        // The request bucket is untouched: all 100 requests still pass.
        for i in 0..RATE_LIMIT_MAX_TOKENS {
            assert!(
                buckets.admit(Lane::Request, now),
                "request {i} within 100/s"
            );
        }
        assert!(!buckets.admit(Lane::Request, now));
        // Spent requests do not refuse frames either, and a second refills.
        assert!(!buckets.admit(Lane::MediaUp, now));
        assert!(buckets.admit(Lane::MediaUp, now + RATE_LIMIT_REFILL_MS));
        assert!(
            buckets.admit(Lane::DaemonMedia, now),
            "daemon media is exempt"
        );
    }
}
