//! This crate's error taxonomy: Tier 1's primitives, plus what only
//! this crate knows about.
//!
//! It used to restate all seven of `rekindle_types::error::CryptoError`
//! under `-Error`-suffixed names, five of them with byte-identical
//! `#[error(...)]` strings. The cost was not the duplication itself but
//! `group::mek_distribution::map_err` — a seven-arm identity function
//! whose entire job was translating one taxonomy into its own copy, and
//! which had to be kept in sync by hand every time either side gained a
//! variant.
//!
//! The layering is the standard one: the lower tier owns the primitive
//! failures, this crate wraps them and adds the concepts Tier 1 must not
//! know about. `CryptoError::PreKeyError` is gone with it — it had zero
//! construction sites anywhere in the workspace.
//!
//! Signal session state stays here rather than moving down because Tier
//! 1 is `serde` + `thiserror` only: teaching it "session" and "prekey"
//! would drag protocol vocabulary into the type crate to save three
//! variants.

use thiserror::Error;

/// The primitive crypto failures, owned by Tier 1 and shared with
/// `rekindle-secrets` and `rekindle-codec`.
pub use rekindle_types::error::CryptoError as CoreCryptoError;

#[derive(Debug, Error)]
pub enum CryptoError {
    /// Key generation, signing, verification, encryption, decryption,
    /// invalid key material, or storage — the seven Tier 1 owns.
    #[error(transparent)]
    Core(#[from] CoreCryptoError),

    /// Signal session state could not be established or advanced.
    #[error("signal session error: {0}")]
    SessionError(String),

    /// No session exists for the requested peer, in the in-memory cache
    /// or in persistent storage.
    #[error("no session for peer {0}")]
    NoSession(String),

    /// Vault is locked (passphrase not yet entered); persistent
    /// load/store of session state is unavailable until unlock.
    #[error("vault locked — session persistence unavailable")]
    VaultLocked,
}

/// Constructors for the wrapped Tier 1 variants.
///
/// These exist so call sites read as they did before the wrap —
/// `CryptoError::encryption(msg)` rather than
/// `CryptoError::Core(CoreCryptoError::Encryption(msg))` — which keeps
/// the seven-fold nesting out of ninety-odd call sites without giving
/// this crate a second copy of the enum.
///
/// They take `String` rather than `impl Into<String>` deliberately:
/// the call sites they replaced were tuple-variant constructions, many
/// written as `CryptoError::decryption("...".into())`. A generic
/// parameter leaves that `.into()` with no inferable target (E0283),
/// so the concrete type is what keeps this a pure rename.
impl CryptoError {
    pub fn key_generation(msg: String) -> Self {
        CoreCryptoError::KeyGeneration(msg).into()
    }

    pub fn signing(msg: String) -> Self {
        CoreCryptoError::Signing(msg).into()
    }

    pub fn verification(msg: String) -> Self {
        CoreCryptoError::Verification(msg).into()
    }

    pub fn encryption(msg: String) -> Self {
        CoreCryptoError::Encryption(msg).into()
    }

    pub fn decryption(msg: String) -> Self {
        CoreCryptoError::Decryption(msg).into()
    }

    pub fn invalid_key(msg: String) -> Self {
        CoreCryptoError::InvalidKey(msg).into()
    }

    pub fn storage(msg: String) -> Self {
        CoreCryptoError::Storage(msg).into()
    }
}
