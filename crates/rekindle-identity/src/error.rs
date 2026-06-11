//! Identity errors — derivation failures, invalid keys, state violations,
//! wire encoding/decoding, and trust/grant chain verification failures.
//!
//! Single error surface for the entire crate. No error variant carries
//! seed material, full keys, or label contents — roots render via the
//! Debug short form only.

/// The identity crate's error type.
///
/// Every fallible operation in this crate returns `Result<T, IdentityError>`.
/// Variants are grouped by subsystem but share one enum to avoid nested
/// error wrapping. The `#[non_exhaustive]` attribute allows adding variants
/// in minor releases without breaking downstream matches.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IdentityError {
    // ── Root / key validation ────────────────────────────────────

    /// Ed25519 public key bytes failed point validation (off-curve,
    /// low-order, or non-canonical encoding).
    #[error("invalid Ed25519 point")]
    InvalidRoot,

    /// X25519 public key derivation or validation failed.
    #[error("invalid X25519 key")]
    InvalidDhKey,

    /// Ed25519 keypair derivation from seed failed inside the
    /// ratchet crate's aws-lc-rs path.
    #[error("Ed25519 keypair derivation failed: {reason}")]
    KeypairDerivation { reason: String },

    // ── Signature verification ───────────────────────────────────

    /// A signature did not verify against the expected key.
    /// `domain` names the derivation_tags constant used for signing
    /// (e.g., "rekindle identity rotation v1").
    #[error("signature verification failed: {domain}")]
    BadSignature { domain: &'static str },

    // ── Rotation chain ───────────────────────────────────────────

    /// A rotation chain has a gap, reversal, or duplicate epoch.
    #[error("rotation chain broken at epoch {at}")]
    BrokenChain { at: u64 },

    /// A rotation chain exceeds `ROTATION_CHAIN_MAX` links.
    #[error("rotation chain exceeds maximum length ({max} links)")]
    ChainTooLong { max: usize },

    /// A rotation proof's `old_root` does not match the expected anchor.
    #[error("rotation chain anchor mismatch")]
    ChainAnchorMismatch,

    // ── Delegation / grant ───────────────────────────────────────

    /// A delegation chain exceeds `GRANT_CHAIN_MAX_DEPTH`.
    #[error("grant chain exceeds depth limit ({max})")]
    GrantChainTooDeep { max: u8 },

    /// A presented grant's epoch is below the known epoch for that
    /// (delegator, delegate) edge.
    #[error("grant epoch {presented} below known {known}")]
    StaleGrant { presented: u64, known: u64 },

    /// An intermediate link in a delegation chain lacks the `Delegate`
    /// capability or its `max_depth` is insufficient for the remaining
    /// chain length.
    #[error("capability not delegable along chain")]
    AttenuationViolation,

    /// A grant's `not_after` timestamp is in the past.
    #[error("grant expired")]
    GrantExpired,

    /// A grant's scope widens (e.g., community-scoped → global) instead
    /// of narrowing or maintaining.
    #[error("grant scope widening is prohibited")]
    GrantScopeWidening,

    // ── Session ──────────────────────────────────────────────────

    /// `session_anchor()` was called with the same root for both sides.
    #[error("self-session is undefined")]
    SelfSession,

    // ── Locator ──────────────────────────────────────────────────

    /// A locator string could not be parsed into its substrate form.
    #[error("locator parse: {0}")]
    LocatorParse(String),

    /// A `LocatorRecord`'s epoch is below the locally known epoch for
    /// that peer, indicating a stale or replayed record.
    #[error("stale locator epoch {presented} below {known}")]
    StaleLocator { presented: u64, known: u64 },

    // ── Prekey binding ───────────────────────────────────────────

    /// A `PrekeyBundleBinding`'s `issued_at` predates the current
    /// rotation epoch's issuance — the bundle was published before the
    /// most recent rotation and is no longer valid.
    #[error("prekey binding predates current rotation epoch")]
    StaleBundle,

    // ── Revocation / termination ─────────────────────────────────

    /// An operation was attempted against a revoked identity.
    #[error("revoked identity")]
    Revoked,

    /// An operation was attempted against a dead (voluntarily terminated)
    /// identity.
    #[error("terminated identity")]
    Terminated,

    // ── Trust store lookup ─────────────────────────────────────────

    /// A trust store operation referenced a peer that is not in the store.
    #[error("peer not in trust store")]
    PeerNotInStore,

    /// A trust store record disappeared between two operations within
    /// the same lock scope — invariant violation.
    #[error("trust record disappeared under lock")]
    RecordDisappeared,

    // ── Prekey binding ───────────────────────────────────────────

    /// A prekey bundle's content hash does not match the binding's
    /// committed hash — the bundle was modified after signing.
    #[error("prekey bundle hash mismatch")]
    BundleHashMismatch,

    // ── Wire encoding / decoding ─────────────────────────────────

    /// The RID/1 CBOR encoder or decoder encountered a structural error:
    /// non-minimal integer width, indefinite length, forbidden major type,
    /// trailing bytes, nesting exceeded, element count exceeded, or
    /// invalid UTF-8.
    #[error("RID/1 encoding: {0}")]
    Encoding(String),

    // ── Vault label ──────────────────────────────────────────────

    /// A constructed label violates the grammar rules (charset, length).
    /// This is always a crate-internal defect — callers cannot trigger
    /// it through the public API because Label is sealed.
    #[error("label grammar violation (internal defect)")]
    LabelGrammar,

    // ── Pseudonym / linkage ──────────────────────────────────────

    /// Pseudonym derivation failed (keypair generation from pseudonym seed).
    #[error("pseudonym derivation failed: {reason}")]
    PseudonymDerivation { reason: String },

    /// A `LinkageProof` has a missing or invalid signature in one or
    /// both directions.
    #[error("linkage proof invalid: {reason}")]
    LinkageInvalid { reason: String },
}
