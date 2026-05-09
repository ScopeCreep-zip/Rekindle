//! Session state types for the Triple Ratchet.
//!
//! `TripleRatchetSession` is the root type serialized to CBOR by the
//! node crate and persisted via `rekindle-storage`. This crate defines
//! the types; the node crate owns serialization and persistence.

pub mod skipped;

use serde::{Deserialize, Serialize};
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

/// Header-encrypted Double Ratchet state.
#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub struct DoubleRatchetState {
    /// Our current DH ratchet private key (X25519 scalar, 32 bytes).
    pub dhs_priv: Option<Zeroizing<[u8; 32]>>,
    /// Our current DH ratchet public key.
    #[zeroize(skip)]
    pub dhs_pub: [u8; 32],
    /// Remote DH ratchet public key. None for responder before first receive.
    #[zeroize(skip)]
    pub dhr_pub: Option<[u8; 32]>,
    /// Root key.
    pub rk: Zeroizing<[u8; 32]>,
    /// Sending chain key. None until first DH step after PQXDH.
    pub cks: Option<Zeroizing<[u8; 32]>>,
    /// Receiving chain key. None until first inbound message.
    pub ckr: Option<Zeroizing<[u8; 32]>>,
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
    pub hks: Option<Zeroizing<[u8; 32]>>,
    /// Current receiving header key.
    pub hkr: Option<Zeroizing<[u8; 32]>>,
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
    /// Initialize the initiator's DR state from the PQXDH session key.
    ///
    /// The initiator (Alice) knows the responder's signed prekey (SPK_B)
    /// from the bundle. She sets `dhr_pub = Some(spk_b)` and performs an
    /// immediate DH ratchet step to derive her sending chain.
    pub fn init_initiator(
        sk: &Zeroizing<[u8; 32]>,
        our_dh_seed: Zeroizing<[u8; 32]>,
        our_dh_pub: [u8; 32],
        their_spk: [u8; 32],
    ) -> Result<Self, crate::error::RatchetError> {
        use crate::crypto::{dh, kdf};

        // DH ratchet step: our initial key × their SPK → root + sending chain + header keys
        let our_key = dh::reusable_from_seed(&our_dh_seed)?;
        let dh_out = dh::ratchet_agree(&our_key, &their_spk)?;
        let (rk, cks, nhks) = kdf::kdf_rk_he(sk, &dh_out)?;

        // Second DH step for initial header keys
        let (rk2, _ckr_unused, nhkr) = kdf::kdf_rk_he(&rk, &dh_out)?;

        Ok(Self {
            dhs_priv: Some(our_dh_seed),
            dhs_pub: our_dh_pub,
            dhr_pub: Some(their_spk),
            rk: rk2,
            cks: Some(cks),
            ckr: None,
            n_send: 0,
            n_recv: 0,
            pn: 0,
            hks: None,
            hkr: None,
            nhks,
            nhkr,
        })
    }

    /// Initialize the responder's DR state from the PQXDH session key.
    ///
    /// The responder (Bob) does NOT know Alice's ephemeral DH public key
    /// at init time — he discovers it from the first inbound message header.
    /// `dhr_pub` is `None`, `cks` is `None` (can't send until DH ratchet).
    pub fn init_responder(
        sk: Zeroizing<[u8; 32]>,
        our_spk_seed: Zeroizing<[u8; 32]>,
        our_spk_pub: [u8; 32],
    ) -> Result<Self, crate::error::RatchetError> {
        use crate::crypto::kdf;

        // Derive initial header keys from SK (no DH step yet — waiting for Alice's first message)
        let zero_dh = Zeroizing::new([0u8; 32]);
        let (_rk_unused, _ck_unused, nhks) = kdf::kdf_rk_he(&sk, &zero_dh)?;
        let (_rk_unused2, _ck_unused2, nhkr) = kdf::kdf_rk_he(&sk, &zero_dh)?;

        Ok(Self {
            dhs_priv: Some(our_spk_seed),
            dhs_pub: our_spk_pub,
            dhr_pub: None,
            rk: sk,
            cks: None,
            ckr: None,
            n_send: 0,
            n_recv: 0,
            pn: 0,
            hks: None,
            hkr: None,
            nhks,
            nhkr,
        })
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
