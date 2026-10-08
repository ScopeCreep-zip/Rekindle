//! SMPL record sizing constant.
//!
//! The `DHTSchema` builders that this module used to host (`community_smpl_schema`,
//! `bootstrap_dflt_schema`, `personal_sync_dflt_schema`) construct `veilid-core`
//! types, so they live in `rekindle-protocol::dht::schema` (a `veilid-core`
//! boundary crate) — keeping `rekindle-records` free of `veilid-core` (Invariant 2).
//! Only the pure, veilid-free sizing constant remains here.

/// Maximum members per SMPL record segment.
/// Veilid allows up to 1024 total subkeys. With m_cnt=1 per member,
/// 255 members = 255 subkeys, well within limits.
pub const MAX_MEMBERS_PER_SEGMENT: usize = 255;
