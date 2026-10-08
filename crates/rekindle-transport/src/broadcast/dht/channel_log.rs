//! The append-only `DhtLog` (spine + segments) used for per-peer DM logs.
//!
//! `ChannelLogOps` used to live here: a second implementation of the
//! per-channel SMPL record's member reads and writes, over this crate's
//! raw `RoutingContext`. It had no caller, and its wire format diverged:
//! it wrote a bare JSON `ChannelMessage` into a member subkey, where
//! `rekindle_protocol::dht::community::channel_record` writes W26-signed
//! pages, so any write through it would have corrupted that member's page.
//! Channel records are `channel_smpl` over `channel_record`, through the
//! record pool (plan C7.4).

// ── Append-only log ──────────────────────────────────────────────────
//
// `DhtLog` is `rekindle_protocol`'s `DHTLog`, not a second copy of it.
// This crate carried a parallel implementation — same LogSpine fields,
// same DEFAULT_SEGMENT_CAPACITY, same spine+segments algorithm, with
// `append()` line-for-line identical bar the error type — over its own
// duplicate of DHTShortArray underneath. Both wrote the identical wire
// format, so the duplication bought nothing and could only drift; it
// had already started to (transport's `add` clamped an index overflow
// to u32::MAX where protocol returns an error).
//
// `?` lifts ProtocolError into TransportError via the per-variant
// conversion in crate::error.
pub use rekindle_protocol::dht::log::DHTLog as DhtLog;
