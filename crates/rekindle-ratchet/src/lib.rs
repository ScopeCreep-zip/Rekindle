//! Triple Ratchet implementation for the Rekindle messaging platform.
//!
//! Implements PQXDH rev 2 (X25519 + ML-KEM-768 hybrid key agreement),
//! the Double Ratchet with Header Encryption (HE-DR), and the Sparse
//! Post-Quantum Ratchet (SPQR) via the ML-KEM Braid protocol.
//!
//! **Backend:** `aws-lc-rs` 1.16 — FIPS-capable via the `regulatory-fips`
//! feature. SOTA default uses the non-FIPS `aws-lc-sys` backend.
//!
//! **No I/O.** This crate performs cryptographic operations only. It does
//! not persist state, open files, or touch the network. The node crate
//! serializes session state (CBOR) and persists it via `rekindle-storage`.
//!
//! **No async.** Every operation is synchronous. CPU-bound operations
//! (ML-KEM keygen, encaps, decaps) should be wrapped in
//! `tokio::task::spawn_blocking` by the caller.

pub mod error;
pub mod crypto;
pub mod ratchet;
pub mod session;
pub mod pqxdh;
pub mod safety;
pub mod wire;

pub use error::RatchetError;
pub use session::{TripleRatchetSession, DoubleRatchetState, Direction};
pub use pqxdh::bundle::PreKeyBundle;
