//! Transport provider registry for the Rekindle messaging platform.
//!
//! Re-exports the `Transport` trait and `InboundEvent` enum from
//! `rekindle-types` and all enabled backend implementations behind
//! feature flags.

pub use rekindle_types::transport::{
    Transport, InboundEvent, TransportError, TransportEvent,
    TransportResult, RecordSchema, BroadcastReport, WatchToken,
    Durability, DeliveryReport,
    TransferProgress, TransferDirection, TransferStatus,
};

// ── Backend providers (feature-gated) ──────────────────────────────

#[cfg(feature = "veilid")]
pub mod veilid {
    pub use rekindle_transport_veilid::*;
}

#[cfg(feature = "ipc")]
pub mod ipc {
    pub use rekindle_transport_ipc::*;
}
