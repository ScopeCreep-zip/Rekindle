//! Phase 14 — unified error type for call signaling + state operations.
//!
//! Distinct from [`crate::CallKeyError`] (which covers only the X25519
//! ECDH key derivation) — this enum carries the broader set of
//! signaling, state-machine, persistence, and transport errors the
//! Phase 14 signaling module returns. The two errors will be
//! consolidated once all callers migrate; for now both coexist for
//! backward compatibility.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CallError {
    // --- Key derivation (forwarded from CallKeyError boundary) ---
    #[error("invalid public key length: expected 32 bytes, got {0}")]
    InvalidPublicKey(usize),
    #[error("hkdf expand failed: {0}")]
    Hkdf(String),

    // --- Identity / state ---
    #[error("identity not loaded")]
    IdentityNotLoaded,
    #[error("call not found: {0}")]
    CallNotFound(String),
    #[error("invalid call state: {0}")]
    InvalidState(String),
    #[error("call already exists for peer: {0}")]
    CallAlreadyExists(String),

    // --- Signaling (wire envelope shapes + glare) ---
    #[error("malformed signaling payload: {0}")]
    MalformedPayload(String),
    #[error("glare resolution: peer {0}")]
    GlareResolved(String),
    #[error("peer temp-muted")]
    PeerMuted,

    // --- Group call key wrapping ---
    #[error("group key wrap failed: {0}")]
    GroupKeyWrap(String),
    #[error("group key unwrap failed: {0}")]
    GroupKeyUnwrap(String),

    // --- Transport (via CallSignalingDeps) ---
    #[error("transport: {0}")]
    Transport(String),
    #[error("peer route unavailable for {0}")]
    PeerRouteUnavailable(String),

    // --- Validation ---
    #[error("invalid input: {0}")]
    InvalidInput(String),

    // --- Voice session bring-up (via CallSignalingDeps) ---
    #[error("voice session: {0}")]
    Session(String),
}

impl CallError {
    /// Convenience constructor for transport-layer errors that arrive
    /// as strings from the Tauri/Veilid boundary.
    pub fn transport<S: Into<String>>(s: S) -> Self {
        Self::Transport(s.into())
    }

    /// Convenience constructor for invalid-input errors.
    pub fn invalid<S: Into<String>>(s: S) -> Self {
        Self::InvalidInput(s.into())
    }
}

impl From<crate::CallKeyError> for CallError {
    fn from(err: crate::CallKeyError) -> Self {
        match err {
            crate::CallKeyError::InvalidPublicKey(n) => CallError::InvalidPublicKey(n),
            crate::CallKeyError::Hkdf(s) => CallError::Hkdf(s),
        }
    }
}

impl From<crate::group::GroupKeyError> for CallError {
    fn from(err: crate::group::GroupKeyError) -> Self {
        // GroupKeyError variants don't include explicit wrap/unwrap
        // directionality at the enum level; map all of them to the
        // unified wrap-failure variant with the original Display string.
        CallError::GroupKeyWrap(err.to_string())
    }
}
