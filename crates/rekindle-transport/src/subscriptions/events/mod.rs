//! Subscription event types — re-exported from `rekindle-types`.
//!
//! The canonical definitions live in `rekindle_types::subscription_events`
//! so both `rekindle-transport` (producer) and `rekindle-cli` (consumer)
//! can use them without the CLI depending on the transport crate.

pub use rekindle_types::subscription_events::*;
