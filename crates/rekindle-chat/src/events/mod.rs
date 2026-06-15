//! Inbound event routing, deduplication, and reactive state management.
//!
//! The event pipeline:
//! 1. Transport sends `InboundEvent::Message` via mpsc channel
//! 2. `router.rs` `run_inbound_loop` reads events, parses TypeId,
//!    verifies signatures, dispatches to services
//! 3. The service constructs a `SubscriptionEvent`
//! 4. `state_effects::apply` updates reactive state (unread, typing, presence)
//! 5. `dedup::EventDedup::check` suppresses duplicates from parallel tiers
//! 6. The event is emitted to the IPC bus for the TUI
//!
//! Steps 4-6 ensure all clients (CLI, TUI, desktop, web, mobile)
//! receive reactive updates without user action and without duplicate
//! noise from the 3-tier watch+gossip+poll delivery.

pub mod registry;
pub mod router;
pub mod conversions;
pub mod dedup;
pub mod state;
pub mod state_effects;
pub mod pipeline;
