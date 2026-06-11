//! Trust state management — per-peer verification lifecycle.
//!
//! The four-state machine (`Pinned`, `PinViolation`, `Verified`,
//! `VerificationViolation`) with the `PreviouslyVerified` latch,
//! backed by a DashMap-sharded store with lock-free reads.
//!
//! Event emission: the store emits `IdentityStatusChange` events for
//! significant trust transitions (violations, their resolution).
//! Insignificant changes (Pinned→Verified) are not emitted.

pub mod state;
pub mod record;
pub mod store;

pub use state::{TrustState, TrustEvent};
pub use record::{TrustRecord, TrustRecordPersist};
pub use store::{TrustStore, IdentityStatusChange};
