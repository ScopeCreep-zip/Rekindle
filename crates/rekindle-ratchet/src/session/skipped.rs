//! Skipped message key types and constants.
//!
//! In HE-DR, skipped keys are indexed by `(header_key, counter)` because
//! the receiver doesn't know the sender's DH public key until after
//! header decryption. The actual storage is in `rekindle-storage` —
//! this module defines the types and constants.

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Maximum skipped keys per chain before the ratchet refuses to advance.
pub const MAX_SKIP_PER_CHAIN: u32 = 1000;

/// Maximum total skipped keys per session.
pub const MAX_SKIP_PER_SESSION: u32 = 2000;

/// Skipped keys expire after 7 days even if `MAX_SKIP` hasn't been reached.
/// Cremers/Jacomme/Naska USENIX '23: stale sessions enable clone attacks.
pub const SKIP_TTL_SECS: i64 = 7 * 86400;

/// A skipped message key with its lookup index.
#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub struct SkippedKey {
    /// The header key that was active when this key was skipped.
    pub header_key: [u8; 32],
    /// The message counter within that header key's chain.
    #[zeroize(skip)]
    pub counter: u32,
    /// The derived message key (single-use, consumed on decrypt).
    pub message_key: [u8; 32],
    /// Unix timestamp when this key was created (for TTL enforcement).
    #[zeroize(skip)]
    pub created_at: i64,
}

/// Origin of a skipped key — EC chain or SPQR chain.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Zeroize)]
pub enum SkipOrigin {
    EcChain,
    SpqrChain,
}
