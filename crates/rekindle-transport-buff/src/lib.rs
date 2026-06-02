//! Lock-free buffer primitives for parallel reorder, pool, and credit-bounded pipelines.
//!
//! This crate provides five primitives and three trait seams that eliminate
//! choke-point-class mistakes by construction:
//!
//! - [`ReorderRing`] — sequence-keyed reorder buffer, MP-fill / SC-drain
//! - [`SlabPool`] — closed-loop reusable-slab pool with return-gated reclaim
//! - [`CreditGuard`] — open-loop atomic CAS credit counter with ceiling
//! - [`DispatchQueue`] — bounded MPMC fan-out to workers
//! - [`Resequencer`] — composed dispatch→parallel→reorder unit
//!
//! The crate is transport-, runtime-, and crypto-agnostic. It compiles and
//! passes its test suite with zero dependency on tokio, io-uring, snow,
//! aws-lc-rs, blake3, or any rekindle wire type.
//!
//! # Litmus test
//!
//! `cargo tree --no-default-features` must show none of the forbidden crates.
//! This is CI-enforced acceptance criterion #1.

#![forbid(unsafe_op_in_unsafe_fn)]
#![deny(missing_docs, unused_imports, dead_code)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names,
    clippy::cast_possible_truncation
)]

mod loom_shim;

pub mod reorder;
pub mod pool;
pub mod credit;
pub mod dispatch;
pub mod resequencer;
pub mod traits;

#[cfg(feature = "seen-set")]
pub mod seen;

#[cfg(feature = "tokio")]
pub mod adapters {
    //! Runtime-specific [`WakeSink`](crate::traits::WakeSink) adapters.
    pub mod tokio;
}

// Re-exports: the public API surface.
pub use crate::reorder::{ReorderRing, PublishError};
pub use crate::pool::{SlabPool, SlabGuard};
pub use crate::credit::CreditGuard;
pub use crate::dispatch::DispatchQueue;
pub use crate::resequencer::{Resequencer, ResequencerFull};
pub use crate::traits::{ReturnGate, SlotLifecycle, WakeSink};
pub use crate::traits::{ImmediateReturn, NoOpLifecycle, SpinWake};

#[cfg(feature = "seen-set")]
pub use crate::seen::OpaqueSeenSet;
