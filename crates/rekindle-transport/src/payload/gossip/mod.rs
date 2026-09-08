//! Gossip broadcast payload types.
//!
//! The outer [`SignedGossipEnvelope`] carries community routing metadata
//! (community_id, sender_pseudonym, TTL, Lamport timestamp) and an Ed25519
//! signature. The inner [`GossipPayload`] is the deserialized content.

mod envelope_into_event;
pub(crate) mod into_event_control;
pub(crate) mod into_event_control_rest;

pub use envelope_into_event::envelope_into_event;

// ── SubscriptionEvent conversion ───────────────────────────────────────
//
// Lives in `envelope_into_event.rs` and maps `CommunityEnvelope`, the
// canonical Cap'n Proto type. `GossipPayload::into_event` used to do it
// against this module's postcard near-copy — 51 of the canonical enum's
// 70 control variants, several of them lossy — which is why a desktop
// peer's reactions, pins and events were unreadable on this track.
