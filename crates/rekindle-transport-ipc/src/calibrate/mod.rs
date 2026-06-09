//! Performance calibration module — structured measurement identifiers,
//! dependency-gated execution, physics-derived validation, and dataset-grade
//! record emission.
//!
//! Implements the `osc:` calibration identifier grammar: a versioned,
//! canonicalizable, machine- and human-readable string that names a single
//! physical performance measurement by encoding its quantity, operation,
//! conditions, and substrate.
//!
//! # Usage
//!
//! ```rust,ignore
//! use rekindle_transport_ipc::calibrate::{OscId, Profile, CalibratedSession};
//!
//! // Parse and canonicalize
//! let id: OscId = "osc:bw/mem.copy?size=16k&cache=l1".parse().unwrap();
//! assert_eq!(id.canonical(), "osc:bw/mem.copy?cache=l1&size=16k");
//!
//! // Dependency-gated session
//! let mut session = CalibratedSession::new(Profile::L4Validation);
//! let missing = session.check_deps(&id);
//! if missing.is_empty() {
//!     session.record(&id, 226.48, 225.53, 227.25, 20);
//! }
//! ```
//!
//! # Module layout (accumulative dependency order)
//!
//! Primitives (no internal deps):
//! - [`quantity`] — physical dimensions (bandwidth, latency, rate, cost)
//! - [`operation`] — hierarchical operation paths with `+` composition
//! - [`condition`] — typed key=value measurement conditions
//! - [`substrate`] — hardware/OS/runtime triple
//! - [`profile`] — conformance levels L0–L5
//!
//! Platform detection:
//! - [`platform`] — runtime CPU/cache/kernel auto-detection
//!
//! Vocabulary management:
//! - [`registry`] — extension validation for operations and conditions
//!
//! Core grammar (depends on primitives):
//! - [`grammar`] — identifier parser, canonical form, equivalence
//!
//! Graph and validation (depends on grammar):
//! - [`dependency`] — accumulative order enforcement
//! - [`validate`] — physics-derived bound checking
//!
//! Records and harness (depends on everything above):
//! - [`record`] — measurement record wire schema
//! - [`harness`] — criterion integration with dependency gating

// Primitives — no internal deps
pub mod quantity;
pub mod operation;
pub mod condition;
pub mod substrate;
pub mod profile;

// Platform detection
pub mod platform;

// Vocabulary management
pub mod registry;

// Core grammar
pub mod grammar;

// Graph and validation
pub mod dependency;
pub mod validate;

// Records, harness, and baseline
pub mod record;
pub mod harness;
pub mod baseline;

// Public API — only types a bench author or downstream consumer names directly
pub use grammar::{OscId, ParseError};
pub use quantity::Quantity;
pub use profile::Profile;
pub use record::CalibrationPoint;
pub use harness::CalibratedSession;
pub use substrate::Substrate;
pub use dependency::DependencyGraph;
pub use baseline::Baseline;

// Pure helpers (no criterion dependency)
pub use harness::fmt_mag;

// Commoditized criterion integration — bench authors use these instead of
// manually configuring criterion and wiring harness coordinates.
// Gated behind bench-harness feature (criterion + tracing-subscriber).
#[cfg(feature = "bench-harness")]
pub use harness::{
    calibrated_bench, calibrated_criterion, bench_contended,
    init_tracing, finalize, workspace_target_dir,
};
