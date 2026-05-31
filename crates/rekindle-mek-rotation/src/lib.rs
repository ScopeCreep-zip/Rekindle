//! Phase 17 — cascade MEK rotation protocol.
//!
//! Architecture §10.5 — when a member leaves a community, the
//! remaining peers must rotate the Media Encryption Key so the
//! departed member can no longer decrypt new gossip messages. The
//! rotation is **decentralised**: every online peer independently
//! computes the same deterministic "rotator" via
//! `blake3(departed_pseudonym || self_pseudonym)` — the peer with
//! the lowest hash performs the rotation; everyone else waits a
//! grace window and cascades down the ordered list if the elected
//! peer is unreachable.
//!
//! This crate hosts:
//! * `election` — cascade-ranking + delay computation
//! * `cascade` — fallback chain (currently colocated with election)
//! * `distribute` — per-member MEK-wrap + gossip broadcast
//! * `cache` — `ChannelMekCache` trait (in-memory MEK lookup)
//! * `persist` — `MekPersist` trait + SqliteMekPersist impl
//! * `deps` — `MekDistributeDeps` trait composing the above + I/O
//! * `error` — `MekRotationError`
//! * `event` — `MekRotationEvent` emitted to the UI
//!
//! Parameterised over `MekDistributeDeps` so the src-tauri shell can
//! supply concrete AppState / DbPool / AppHandle / Veilid wiring.

pub mod cache;
pub mod deps;
pub mod distribute;
pub mod election;
pub mod error;
pub mod event;
pub mod receive;
pub mod rotate;

pub use cache::InMemoryMekCache;
pub use deps::{ChannelMekCache, MekDistributeDeps, MekPersist, RotationRecipient};
pub use distribute::{distribute_mek, wait_for_rotation_slot};
pub use election::{cascade_candidates, cascade_delay, select_mek_responder, CASCADE_TIMEOUT_SECS, MAX_CASCADES};
pub use error::MekRotationError;
pub use event::MekRotationEvent;
pub use receive::{handle_incoming_mek_transfer, mek_cache_has_generation, unwrap_received_mek};
pub use rotate::{rotate_text_mek_for_departure, rotate_voice_mek_for_membership};
