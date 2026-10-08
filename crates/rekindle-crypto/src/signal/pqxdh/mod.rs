//! PQXDH — post-quantum key agreement (Signal §PQXDH spec).
//!
//! Phase 3a of the decomposed-harvest plan: builds the cryptographic
//! primitives in isolation. The existing X3DH path in
//! [`super::session::SignalSessionManager`] is unchanged; PQXDH lives
//! next to it, exercisable only via unit tests until Phase 3b plugs the
//! new handshake into `establish_session` / `respond_to_session`.
//!
//! ## Handshake shape (verified against signal.org/docs/specifications/pqxdh/)
//!
//! Prekey bundle published by responder:
//! - `IK_B` — Ed25519 identity verification key
//! - `SPK_B` — X25519 signed prekey, plus Ed25519 signature
//! - `OPK_B` (optional) — X25519 one-time prekey
//! - `PQPK_LR_B` — ML-KEM-768 last-resort prekey, plus Ed25519 signature
//! - `PQPK_OT_B` (optional) — ML-KEM-768 one-time prekey, plus signature
//!
//! Initiator (Alice):
//! 1. Verify SPK signature against `IK_B`
//! 2. Verify chosen PQ key signature against `IK_B`
//! 3. Generate ephemeral `EK_A` (X25519 `StaticSecret` so it can DH four times)
//! 4. `DH1 = DH(IK_A, SPK_B)`
//! 5. `DH2 = DH(EK_A, IK_B)`
//! 6. `DH3 = DH(EK_A, SPK_B)`
//! 7. `DH4 = DH(EK_A, OPK_B)` if `OPK_B` present
//! 8. `(CT, SS) = ML-KEM-Encap(chosen_PQ_pubkey)` (prefer `PQPK_OT_B`)
//! 9. `root_key = HKDF-SHA256(F || DH1 || DH2 || DH3 [|| DH4] || SS)` with
//!    `F = 0xFF × 32` per the X3DH/PQXDH KDF convention
//!
//! Responder (Bob): symmetric — recover `SS` via ML-KEM decapsulate, derive
//! the same DH outputs in reverse, compute identical `root_key`.

pub mod bundle;
pub mod handshake;
pub mod kdf;
pub mod keys;
pub mod verify;

pub use bundle::PqPreKeyBundle;
pub use handshake::{pqxdh_initiator, pqxdh_responder, InitiatorHandshake, ResponderInput};

#[derive(Debug, thiserror::Error)]
pub enum PqxdhError {
    #[error("invalid X25519 key bytes (expected 32, got {0})")]
    InvalidX25519Length(usize),
    #[error("Ed25519 → X25519 conversion failed: {0}")]
    EdToX25519(String),
    #[error("invalid ML-KEM public key (expected 1184 bytes)")]
    InvalidMlKemPublic,
    #[error("invalid ML-KEM ciphertext (expected 1088 bytes)")]
    InvalidMlKemCiphertext,
    #[error("Ed25519 signature verification failed: {0}")]
    SignatureVerify(String),
    #[error("HKDF expand failed: {0}")]
    Hkdf(String),
    #[error("PQ prekey bundle missing both one-time and last-resort PQ keys")]
    NoPqKey,
}
