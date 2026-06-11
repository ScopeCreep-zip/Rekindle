//! Layer 1 — Root. The `IdentityRoot`, `PeerRef`, rotation chain
//! verification, revocation, and termination.
//!
//! `IdentityRoot` and `RotationEpoch` are defined in `origin::originate`
//! (they are produced at origination). This module re-exports them and
//! adds the chain-verification, revocation-verification, and death-notice
//! types that operate on roots post-origination.

pub mod rotation;
pub mod termination;

// Re-exports from origin — canonical import path is crate::root::*
pub use crate::origin::originate::{
    IdentityRoot, PeerRef, RotationEpoch, RevocationCertificate,
};

use std::time::Duration;

/// Old-root-signed material remains acceptable for this window after
/// a peer observes a valid `RotationProof`. Hard cut afterward.
/// Voided immediately by a `RevocationCertificate`.
pub const ROTATION_GRACE: Duration = Duration::from_secs(14 * 24 * 3600); // 14 days

/// Maximum number of links in a presented rotation chain.
pub const ROTATION_CHAIN_MAX: usize = 64;
