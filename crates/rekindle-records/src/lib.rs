//! DHT record lifecycle management for Rekindle v2.0.
//!
//! Tier 3 — wraps veilid-core DHT API with the v2.0 universal SMPL schema
//! (`o_cnt: 0`, 255 member slots) and retry logic for durable writes.
//!
//! All community records (governance, registry, channels) use the same
//! schema — the Q-pid equation from architecture doc §4.2.

pub mod lifecycle;
pub mod retry;
pub mod schema;
