#![forbid(unsafe_code)]
//! Reusable event primitives extracted from `rekindle-transport::subscriptions`.
//!
//! At HEAD, `rekindle-transport::SubscriptionManager::process_event`
//! (`crates/rekindle-transport/src/subscriptions/mod.rs:274-294`) implemented
//! the enrich → state_effects::apply → dedup::check → broadcast pipeline
//! using local sibling modules (`state.rs`, `state_effects.rs`, `dedup.rs`).
//! Phase 1 of the decomposed-harvest plan
//! (`/Users/kali/.claude/plans/memoized-dazzling-torvalds.md` § Phase 1)
//! hoists those three modules into this crate so other consumers — currently
//! src-tauri's Phase 10 `event_resume` Tauri command — can use them without
//! pulling in `rekindle-transport`'s `veilid-core` dep.
//!
//! Transport keeps its `SubscriptionManager` (which IS the pipeline). This
//! crate publishes the building blocks plus the host's one
//! [`SessionJournal`] (plan D3.2: epoch + sequence cursors, replay and
//! resume for every frontend) and the backend's typing state ([`typing`],
//! plan E2.3: the receiver's expiry, the sender's refresh and idle stop).

pub mod dedup;
pub mod journal;
pub mod notify_policy;
pub mod state;
pub mod state_effects;
pub mod typing;

pub use dedup::EventDedup;
pub use journal::{EventStream, SessionJournal};
pub use state::SubscriptionState;
