//! GoogCC send-side bandwidth estimation and a leaky-bucket pacer.
//!
//! This crate is a copy of str0m 0.24.1's `src/bwe/` and `src/pacer/` (revision
//! `393a3d23`, MIT OR Apache-2.0, used under MIT; see `NOTICE`), with the
//! supporting types those modules import from elsewhere in str0m. The changes
//! are mechanical: paths, visibility, the RTP-stack identity of a queue
//! (`MidRid` → [`QueueId`]) and lint-driven rewrites that keep behaviour. The
//! algorithms, constants and unit tests are str0m's (ADR 0015, plan E4.3.3),
//! with one behavioral difference: periodic probing while application-limited
//! is off by default, as in libwebrtc (`ProbeControl` `periodic_alr_probing`).
//!
//! - [`Bwe`]: the estimator (trendline delay detector, loss controller,
//!   acked-bitrate estimator, probe controller and estimator, ALR detector,
//!   link-capacity estimator). Feed it [`TwccSendRecord`]s built from transport
//!   feedback; read [`Bwe::poll_estimate`] / [`Bwe::last_estimate`].
//! - [`LeakyBucketPacer`]: decides when the next packet may leave and which
//!   queue it comes from. Unpaced queues (audio) go first, then media queues,
//!   then padding. [`SendQueue`] is str0m's per-stream queue that produces the
//!   [`QueueSnapshot`]s the pacer reads.
//! - [`PacerControl`]: derives the pacing and padding rates from the estimate.
//! - [`allocation`]: splits a route's estimate between audio and video, after
//!   libwebrtc's `BitrateAllocator` (Rekindle's own module, not str0m's).
//!
//! Everything is Sans-I/O: time only advances through the `Instant`s the caller
//! passes in.

#![forbid(unsafe_code)]

/// str0m's `log_stat!`, which writes to stdout only under str0m's
/// `_internal_dont_use_log_stats` feature and otherwise expands to nothing.
///
/// Here it never writes either. The arguments go into a closure that is never
/// called, so they are type-checked and count as used (str0m silences the
/// resulting unused bindings with `#[allow(unused)]`, which this workspace
/// forbids) but are never evaluated, exactly as in str0m's default build.
macro_rules! log_stat {
    ($name:expr, $($arg:expr),+) => {
        let _ = || {
            let _ = $name;
            $(let _ = &$arg;)+
        };
    };
}

/// The numeric-id boilerplate from str0m `src/rtp/id.rs` (`num_id!`), without
/// the random constructor.
macro_rules! num_id {
    ($id:ident, $t:tt) => {
        impl Deref for $id {
            type Target = $t;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl From<$t> for $id {
            fn from(v: $t) -> Self {
                $id(v)
            }
        }

        impl fmt::Display for $id {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

pub mod allocation;
mod bandwidth;
mod bwe;
mod pacer;
mod reason;
mod twcc;
mod util;

pub use bandwidth::{Bitrate, DataSize};
pub use bwe::{Bwe, ProbeClusterConfig, ProbeKind};
pub use pacer::{
    LeakyBucketPacer, NullPacer, Pacer, PacerControl, PacerImpl, PacerReason, PacingResult,
    PaddingRequest, QueueId, QueuePriority, QueueSnapshot, QueueState, SendQueue,
};
pub use reason::Reason;
pub use twcc::{TwccClusterId, TwccPacketId, TwccRecvReport, TwccSendRecord, TwccSeq};
