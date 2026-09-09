//! Realtime media measurement — what a call is actually doing.
//!
//! Tier 3, pure logic: no I/O, no async, no `veilid-core`, no Tauri.
//! Every metric is computed from values the caller already holds, so a
//! whole session can be simulated in a unit test.
//!
//! ## Why this exists
//!
//! Before this, "connection quality" was
//! `send_failures / packets_sent` mapped to good/fair/poor — a count of
//! **our own local send errors**. That is not a measurement of the
//! link: it cannot see loss (the sender's `send()` succeeded), cannot
//! see jitter, cannot see a dead link (zero failures over zero packets
//! reads as perfect), and cannot tell a caller whether the problem is
//! the network or their own buffer.
//!
//! Which meant every media tuning decision — bitrate ceilings, jitter
//! targets, route parameters — rested on numbers that did not measure
//! the thing being tuned.
//!
//! ## The model
//!
//! Standard telecom instrumentation, because the problem is old:
//!
//! - [`ReceptionTracker`] — RFC 3550 interarrival jitter, and the
//!   RFC 3611 VoIP Metrics model: loss and discard kept apart, and the
//!   burst/gap split that distinguishes 5 % loss you can hear through
//!   from 5 % loss that ate a word.
//! - [`quality`] — ITU-T G.107 E-model R factor and MOS, plus
//!   [`LinkTracker`](quality::LinkTracker), the state machine a session
//!   branches on: `Good`/`Fair`/`Poor`/`Lost`/`Recovering`.
//!
//! `Recovering` is a state rather than a return to `Good` because
//! recovery has to be *acted on*: a video sender must emit a keyframe,
//! since the peer's decoder has a hole in its reference chain.
//!
//! ## What this is not
//!
//! It does not measure Veilid. `VeilidStateNetwork` gives node-wide
//! `bps_up`/`bps_down` and per-relay latency, but nothing per-call —
//! our peers are pseudonymous, so there is no mapping from a call
//! participant to a routing-table entry. End-to-end quality has to be
//! measured end-to-end, which is why RTCP exists and why this does the
//! same job.

pub mod quality;
mod reception;

pub use quality::{mos_from_r, r_factor, score, LinkState, LinkTracker, QualityScore};
pub use reception::{ReceptionMetrics, ReceptionTracker, DEFAULT_GMIN};
