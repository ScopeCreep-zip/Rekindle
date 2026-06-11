//! Trust state management — per-peer verification lifecycle.
//!
//! The four-state machine (`Pinned`, `PinViolation`, `Verified`,
//! `VerificationViolation`) with the `PreviouslyVerified` latch,
//! backed by a DashMap-sharded store with lock-free reads.

pub mod state;
pub mod record;
pub mod store;

pub use state::{TrustState, TrustEvent};
pub use record::{TrustRecord, TrustRecordPersist};
pub use store::TrustStore;
