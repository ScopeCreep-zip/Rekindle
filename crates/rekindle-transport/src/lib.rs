//! Transport provider registry for the Rekindle messaging platform.
//!
//! Re-exports the `Transport` and `TransportCallback` traits from
//! `rekindle-types` and all enabled backend implementations behind
//! feature flags. Consumers depend on this crate only — never on
//! backend crates directly.
//!
//! ```toml
//! # Enable specific backends:
//! rekindle-transport = { features = ["veilid"] }       # default
//! rekindle-transport = { features = ["veilid", "matrix"] }
//! ```

// Re-export the trait ecosystem from rekindle-types
pub use rekindle_types::transport::{
    Transport, TransportCallback, TransportError, TransportEvent,
    TransportResult, RecordSchema, BroadcastReport, WatchToken,
};

// ── Backend providers (feature-gated) ──────────────────────────────

#[cfg(feature = "veilid")]
pub mod veilid {
    pub use rekindle_transport_veilid::*;
}
