//! PreKeyBundle type and PQXDH key material.

use serde::{Deserialize, Serialize};

/// Ed25519 signature (64 bytes) with compile-time length enforcement.
///
/// Serde's default array impls only cover sizes up to 32. This newtype
/// provides `Serialize`/`Deserialize` via byte-string encoding and
/// validates length on deserialization. Internal code uses `[u8; 64]`
/// through `.as_ref()` and `::from_bytes()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature([u8; 64]);

impl Signature {
    pub fn from_bytes(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }
}

impl AsRef<[u8]> for Signature {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Serialize for Signature {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let bytes: Vec<u8> = Deserialize::deserialize(de)?;
        let arr: [u8; 64] = bytes.try_into().map_err(|v: Vec<u8>| {
            serde::de::Error::custom(format!("signature must be 64 bytes, got {}", v.len()))
        })?;
        Ok(Self(arr))
    }
}

/// A published prekey bundle for PQXDH rev 2.
///
/// Every public key is prefixed with its algorithm byte on the wire:
/// - `0x01` for X25519 (32 bytes → 33 bytes on wire)
/// - `0x02` for ML-KEM-768 (1184 bytes → 1185 bytes on wire)
///
/// Signatures cover `alg_prefix || [domain_tag ||] key_bytes`.
///
/// Signature fields are `Vec<u8>` (not `[u8; 64]`) because serde's
/// default array impls only cover sizes up to 32. Length validation
/// (== 64 bytes) is enforced in [`super::verify::validate_bundle`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreKeyBundle {
    /// Ed25519 identity verification key (32 bytes).
    pub ik_ed25519: [u8; 32],
    /// X25519 identity DH public key (32 bytes).
    pub ik_x25519: [u8; 32],
    /// Signed prekey ID.
    pub spk_id: u64,
    /// X25519 signed prekey (32 bytes).
    pub spk: [u8; 32],
    /// Ed25519 signature over `0x01 || spk` (64 bytes).
    pub spk_signature: Signature,
    /// Optional one-time X25519 prekey ID (0 = none).
    pub opk_id: u64,
    /// Optional one-time X25519 prekey (32 bytes).
    pub opk: Option<[u8; 32]>,
    /// ML-KEM-768 one-time PQ prekey ID.
    pub pqpk_ot_id: u64,
    /// ML-KEM-768 one-time encapsulation key (1184 bytes).
    pub pqpk_ot: Vec<u8>,
    /// Ed25519 signature over `0x02 || "OT" || pqpk_ot` (64 bytes).
    pub pqpk_ot_signature: Signature,
    /// ML-KEM-768 last-resort encapsulation key (1184 bytes).
    pub pqpk_lr: Vec<u8>,
    /// Ed25519 signature over `0x02 || "LR" || pqpk_lr` (64 bytes).
    pub pqpk_lr_signature: Signature,
    /// Unix timestamp when this bundle was published.
    pub published_at: i64,
}

impl PreKeyBundle {
    /// Whether this bundle has a one-time X25519 prekey.
    pub fn has_opk(&self) -> bool {
        self.opk_id != 0 && self.opk.is_some()
    }

    /// Whether this bundle has a one-time PQ prekey (vs last-resort only).
    pub fn has_pqpk_ot(&self) -> bool {
        self.pqpk_ot_id != 0 && self.pqpk_ot.len() == 1184
    }

    /// Get the PQ encapsulation key to use (one-time if available, else last-resort).
    /// Returns `(ek_bytes, is_last_resort)`.
    pub fn pq_ek(&self) -> (&[u8], bool) {
        if self.has_pqpk_ot() {
            (&self.pqpk_ot, false)
        } else {
            (&self.pqpk_lr, true)
        }
    }
}

/// PQXDH initiator message — sent from Alice to Bob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PqxdhInitMessage {
    /// Alice's X25519 identity DH public key.
    pub ik_a_x25519: [u8; 32],
    /// Alice's Ed25519 identity verification key.
    pub ik_a_ed25519: [u8; 32],
    /// Alice's X25519 ephemeral public key.
    pub ek_a: [u8; 32],
    /// Which of Bob's signed prekeys was used.
    pub spk_b_id: u64,
    /// Which of Bob's PQ prekeys was used.
    pub pqpk_b_id: u64,
    /// Whether Bob's last-resort PQ prekey was used (F4 disambiguator).
    pub pqpk_is_last_resort: bool,
    /// Which of Bob's one-time X25519 prekeys was used (0 = none).
    pub opk_b_id: u64,
    /// ML-KEM-768 ciphertext (1088 bytes).
    pub kem_ct: Vec<u8>,
    /// Initial AEAD ciphertext (first DR message).
    pub initial_ciphertext: Vec<u8>,
}

/// Result of the PQXDH initiator computation.
pub struct PqxdhInitResult {
    /// The session key (initial root key for the Double Ratchet).
    pub session_key: zeroize::Zeroizing<[u8; 32]>,
    /// The message to send to the responder.
    pub init_message: PqxdhInitMessage,
    /// Initialized EC Double Ratchet state (ready to send/receive).
    pub ec_state: crate::session::DoubleRatchetState,
}

/// Result of the PQXDH responder computation.
pub struct PqxdhResponderResult {
    /// The session key (must match the initiator's).
    pub session_key: zeroize::Zeroizing<[u8; 32]>,
}
