//! Session state types for the Triple Ratchet.
//!
//! `TripleRatchetSession` is the root type serialized to CBOR by the
//! node crate and persisted via `rekindle-storage`. This crate defines
//! the types; the node crate owns serialization and persistence.

pub mod skipped;

use serde::{Deserialize, Serialize};
use tracing::debug;
use zeroize::{ZeroizeOnDrop, Zeroizing};

/// Session direction: who initiated the PQXDH handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// We sent the initial PQXDH message.
    Initiator,
    /// We received the initial PQXDH message.
    Responder,
}

/// Trust level progression after PQXDH and safety number verification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum TrustLevel {
    /// No verification performed.
    #[default]
    Untrusted,
    /// PQXDH completed. `full_fs` = true if OPK was consumed.
    TrustOnFirstUse { full_fs: bool },
    /// Safety number verified out-of-band.
    SafetyNumberVerified { at: i64, method: VerificationMethod },
    /// Safety number verified AND one-time prekeys consumed.
    FullyVerified { at: i64, pq_ots_used: bool },
}


/// How the safety number was verified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationMethod {
    QrCodeScan,
    ManualDigitCompare,
    AudioReadback,
    OutOfBandChannel,
}

/// Header-encrypted Double Ratchet state (Olm model).
///
/// Both chain keys and header keys initialized at session creation.
/// No Option fields — the type system prevents "no sending chain key"
/// errors by construction. Both sides can send immediately after init.
#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub struct DoubleRatchetState {
    /// Our current DH ratchet private key (X25519 scalar, 32 bytes).
    pub dhs_priv: Zeroizing<[u8; 32]>,
    /// Our current DH ratchet public key.
    #[zeroize(skip)]
    pub dhs_pub: [u8; 32],
    /// Remote DH ratchet public key. Always known after init.
    #[zeroize(skip)]
    pub dhr_pub: [u8; 32],
    /// Root key.
    pub rk: Zeroizing<[u8; 32]>,
    /// Sending chain key. Always initialized at session creation.
    pub cks: Zeroizing<[u8; 32]>,
    /// Receiving chain key. Always initialized at session creation.
    pub ckr: Zeroizing<[u8; 32]>,
    /// Sending counter.
    #[zeroize(skip)]
    pub n_send: u32,
    /// Receiving counter.
    #[zeroize(skip)]
    pub n_recv: u32,
    /// Previous sending chain length.
    #[zeroize(skip)]
    pub pn: u32,
    // ── Header encryption keys ──────────────────────────────────
    /// Current sending header key.
    pub hks: Zeroizing<[u8; 32]>,
    /// Current receiving header key.
    pub hkr: Zeroizing<[u8; 32]>,
    /// Next sending header key.
    pub nhks: Zeroizing<[u8; 32]>,
    /// Next receiving header key.
    pub nhkr: Zeroizing<[u8; 32]>,
}

/// ML-KEM Braid state (one-shot bridge).
///
/// The 11 spec states are preserved for cross-implementation mapping.
/// States that collapse in the one-shot bridge are pass-through variants.
#[derive(Serialize, Deserialize)]
pub enum MlKemBraidState {
    /// No ML-KEM keypair sampled yet.
    Idle,
    /// Sender: keypair generated, header ready to chunk.
    KeysSampled {
        epoch: u32,
        #[serde(with = "serde_dk")]
        dk: Zeroizing<[u8; 2400]>,
        ek_seed: [u8; 32],
        ek_vec_hash: [u8; 32],
        ek_vector: Vec<u8>, // 1152 bytes
    },
    /// Sender: header chunks sent, awaiting ek_vector delivery + ct response.
    HeaderSent {
        epoch: u32,
        #[serde(with = "serde_dk")]
        dk: Zeroizing<[u8; 2400]>,
        ek_seed: [u8; 32],
        ek_vec_hash: [u8; 32],
        ek_vector: Vec<u8>,
    },
    /// Receiver: header received, buffering ek_vector chunks.
    EkBuffering {
        epoch: u32,
        ek_seed: [u8; 32],
        ek_vec_hash: [u8; 32],
        ek_vector_partial: Vec<u8>,
        chunks_received: u32,
        chunks_expected: u32,
    },
    /// Receiver: ek_vector complete, encaps done, chunking ciphertext.
    CtSending {
        epoch: u32,
        epoch_ss: Zeroizing<[u8; 32]>,
        ct: Vec<u8>, // 1088 bytes
    },
    /// Sender: reassembling ciphertext chunks.
    CtBuffering {
        epoch: u32,
        #[serde(with = "serde_dk")]
        dk: Zeroizing<[u8; 2400]>,
        ct_partial: Vec<u8>,
        chunks_received: u32,
        chunks_expected: u32,
    },
    /// Epoch complete — shared secret available.
    Complete {
        epoch: u32,
        epoch_ss: Zeroizing<[u8; 32]>,
    },
}

impl Drop for MlKemBraidState {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        match self {
            Self::KeysSampled { dk, ek_vector, .. }
            | Self::HeaderSent { dk, ek_vector, .. } => {
                dk.zeroize();
                ek_vector.zeroize();
            }
            Self::CtSending { epoch_ss, ct, .. } => {
                epoch_ss.zeroize();
                ct.zeroize();
            }
            Self::CtBuffering { dk, ct_partial, .. } => {
                dk.zeroize();
                ct_partial.zeroize();
            }
            Self::Complete { epoch_ss, .. } => {
                epoch_ss.zeroize();
            }
            Self::Idle | Self::EkBuffering { .. } => {}
        }
    }
}

/// Root session type combining EC ratchet + SPQR + metadata.
#[derive(Serialize, Deserialize)]
pub struct TripleRatchetSession {
    /// BLAKE3(IK_A || IK_B || nonce).
    pub session_id: [u8; 32],
    /// Who initiated the PQXDH handshake.
    pub direction: Direction,
    /// EC Double Ratchet with header encryption.
    pub ec: DoubleRatchetState,
    /// ML-KEM Braid state.
    pub spqr: MlKemBraidState,
    /// Whether SPQR has produced at least one epoch secret.
    pub spqr_active: bool,
    /// Current SPQR epoch number.
    pub spqr_epoch: u32,
    /// Trust progression.
    pub trust_level: TrustLevel,
    /// Unix timestamp of last send or receive.
    pub last_active: i64,
    /// Unix timestamp of last successful decrypt. 0 if never decrypted.
    /// Used for session selection (prefer most-recently-decrypted) and
    /// wedge detection (was working → now failing = wedged).
    #[serde(default)]
    pub last_decrypted_at: i64,
    /// Unix timestamp of session creation.
    pub created_at: i64,
}

impl Drop for TripleRatchetSession {
    fn drop(&mut self) {
        // ec: ZeroizeOnDrop handles DoubleRatchetState
        // spqr: manual Drop handles MlKemBraidState
        // remaining fields are non-secret metadata
    }
}

// ── Initialization ─────────────────────────────────────────────────

impl DoubleRatchetState {
    /// Initialize the Initiator's DR state (Olm model).
    ///
    /// Three KDFs from commutative DH:
    ///   KDF1: chain_a (Initiator sends) + hk_a (Initiator header encrypt)
    ///   KDF2: chain_b (Initiator receives) + hk_b (Initiator header decrypt)
    ///   KDF3: nhk_a + nhk_b (next header keys, distinct from current)
    ///
    /// Responder swaps: cks↔ckr, hks↔hkr, nhks↔nhkr.
    pub fn init_initiator(
        sk: &Zeroizing<[u8; 32]>,
        our_dh_seed: Zeroizing<[u8; 32]>,
        our_dh_pub: [u8; 32],
        their_spk: [u8; 32],
    ) -> Result<Self, crate::error::RatchetError> {
        use crate::crypto::{dh, kdf};

        let our_key = dh::reusable_from_seed(&our_dh_seed)?;
        let dh_out = dh::ratchet_agree(&our_key, &their_spk)?;
        let (rk, chain_a, hk_a) = kdf::kdf_rk_he(sk, &dh_out)?;
        let (rk2, chain_b, hk_b) = kdf::kdf_rk_he(&rk, &dh_out)?;
        let (rk3, nhk_a, nhk_b) = kdf::kdf_rk_he(&rk2, &dh_out)?;

        let state = Self {
            dhs_priv: our_dh_seed,
            dhs_pub: our_dh_pub,
            dhr_pub: their_spk,
            rk: rk3,
            cks: chain_a,
            ckr: chain_b,
            n_send: 0,
            n_recv: 0,
            pn: 0,
            hks: hk_a,
            hkr: hk_b,
            nhks: nhk_a,
            nhkr: nhk_b,
        };
        debug!("DR::init_initiator: Olm model — both chains initialized");
        Ok(state)
    }

    /// Initialize the Responder's DR state (Olm model).
    ///
    /// Same three KDFs as Initiator (commutative DH produces same outputs).
    /// Swapped assignment: Responder sends on chain_b, receives on chain_a.
    pub fn init_responder(
        sk: Zeroizing<[u8; 32]>,
        our_spk_seed: Zeroizing<[u8; 32]>,
        our_spk_pub: [u8; 32],
        their_ratchet_dh_pub: [u8; 32],
    ) -> Result<Self, crate::error::RatchetError> {
        use crate::crypto::{dh, kdf};

        let our_key = dh::reusable_from_seed(&our_spk_seed)?;
        let dh_out = dh::ratchet_agree(&our_key, &their_ratchet_dh_pub)?;
        let (rk, chain_a, hk_a) = kdf::kdf_rk_he(&sk, &dh_out)?;
        let (rk2, chain_b, hk_b) = kdf::kdf_rk_he(&rk, &dh_out)?;
        let (rk3, nhk_a, nhk_b) = kdf::kdf_rk_he(&rk2, &dh_out)?;

        let state = Self {
            dhs_priv: our_spk_seed,
            dhs_pub: our_spk_pub,
            dhr_pub: their_ratchet_dh_pub,
            rk: rk3,
            cks: chain_b,               // SWAPPED: Responder sends on chain_b
            ckr: chain_a,               // SWAPPED: Responder receives on chain_a
            n_send: 0,
            n_recv: 0,
            pn: 0,
            hks: hk_b,                  // SWAPPED: encrypts headers with hk_b
            hkr: hk_a,                  // SWAPPED: decrypts headers with hk_a
            nhks: nhk_b,               // SWAPPED
            nhkr: nhk_a,               // SWAPPED
        };
        debug!("DR::init_responder: Olm model — both chains initialized, can send immediately");
        Ok(state)
    }
}

impl TripleRatchetSession {
    /// Create a new session after PQXDH completes.
    pub fn new(
        session_id: [u8; 32],
        direction: Direction,
        ec: DoubleRatchetState,
        trust_level: TrustLevel,
    ) -> Self {
        #[allow(clippy::cast_possible_wrap)]
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        Self {
            session_id,
            direction,
            ec,
            spqr: MlKemBraidState::Idle,
            spqr_active: false,
            spqr_epoch: 0,
            trust_level,
            last_active: now,
            last_decrypted_at: 0,
            created_at: now,
        }
    }
}

/// Serde helper for `Zeroizing<[u8; 2400]>` — serialize as raw bytes.
mod serde_dk {
    use serde::{Deserializer, Serializer};
    use zeroize::Zeroizing;

    pub fn serialize<S: Serializer>(
        dk: &Zeroizing<[u8; 2400]>,
        ser: S,
    ) -> Result<S::Ok, S::Error> {
        ser.serialize_bytes(dk.as_ref())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        de: D,
    ) -> Result<Zeroizing<[u8; 2400]>, D::Error> {
        use serde::de::Error;
        let bytes: Vec<u8> = serde::Deserialize::deserialize(de)?;
        if bytes.len() != 2400 {
            return Err(D::Error::custom(format!(
                "dk must be 2400 bytes, got {}",
                bytes.len()
            )));
        }
        let mut dk = Zeroizing::new([0u8; 2400]);
        dk.copy_from_slice(&bytes);
        Ok(dk)
    }
}
