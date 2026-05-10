//! Inbound event routing, deduplication, and reactive state management.
//!
//! The event pipeline:
//! 1. Transport delivers raw bytes via `TransportCallback::on_message`
//!    or `TransportCallback::on_record_change`
//! 2. `router.rs` parses TypeId, verifies signatures, decrypts, dispatches
//!    to the correct service (messaging, friendship, community)
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
