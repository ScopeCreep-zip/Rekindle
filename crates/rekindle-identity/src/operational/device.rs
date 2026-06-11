//! Layer 2 — Device identity accommodation.
//!
//! Multi-device does not ship in v1. The structures and verification
//! ship; only one device per identity is instantiated, and every
//! signature-producing path uses the root keypair directly.
//!
//! The accommodation contract: every message-verification path accepts
//! EITHER a root signature OR (device signature + presented DeviceRecord
//! chaining to the root at current epoch). Turning multi-device on
//! later is a configuration change, not a structural change.

use rekindle_ratchet::crypto::sign;

use crate::error::IdentityError;
use crate::origin::originate::{IdentityRoot, RotationEpoch};
use crate::origin::tags::derivation_tags;
use crate::wire::encode;
use crate::wire::signable::{Hlc, Signable, Signature64};

/// Device identifier — 16 random bytes, local-meaningful.
///
/// NOT identity (per D-21: nothing verifies against it). Used for
/// local bookkeeping (which device holds which ratchet session).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DeviceId(pub [u8; 16]);

impl DeviceId {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 16];
        getrandom::getrandom(&mut bytes).expect("CSPRNG must be available");
        Self(bytes)
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

/// Ed25519 public key for a specific device, distinct from `IdentityRoot`.
///
/// Sealed newtype — prevents passing an `IdentityRoot` where a device
/// key is expected and vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DeviceSigningKey(pub [u8; 32]);

impl DeviceSigningKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

/// A device subordinate to a user's global identity.
///
/// The principal's root key signs this record. The device key is
/// signed by the root at the claimed epoch — verification chains
/// to the IdentityRoot.
#[derive(Debug, Clone)]
pub struct DeviceRecord {
    /// The Layer 0 identity this device belongs to.
    pub principal: IdentityRoot,
    /// The epoch of the principal when this record was issued.
    pub principal_epoch: RotationEpoch,
    /// This device's Ed25519 public key.
    pub device_key: DeviceSigningKey,
    /// Local device identifier.
    pub device_id: DeviceId,
    /// Human-readable device name.
    pub device_name: String,
    /// When this record was issued.
    pub issued_at: Hlc,
    /// Per (principal, device_id) edge, latest-wins. A higher epoch
    /// with the same device_id supersedes older records.
    pub epoch: u64,
    /// Signature by the principal root over the DEVICE_SIGN domain.
    pub signature: Signature64,
}

struct DeviceSignable<'a> {
    principal_bytes: [u8; 32],
    principal_epoch: u64,
    device_key_bytes: [u8; 32],
    device_id_bytes: [u8; 16],
    device_name: &'a str,
    issued_at: Hlc,
    epoch: u64,
}

impl Signable for DeviceSignable<'_> {
    const SIGN_DOMAIN: &'static str = derivation_tags::DEVICE_SIGN;

    fn encode_fields(&self, buf: &mut Vec<u8>) {
        // WIRE FORMAT: field order pinned — reordering is a wire break
        encode::array_head(buf, 7);
        encode::bytes(buf, &self.principal_bytes);
        encode::unsigned(buf, self.principal_epoch);
        encode::bytes(buf, &self.device_key_bytes);
        encode::bytes(buf, &self.device_id_bytes);
        encode::text(buf, self.device_name);
        self.issued_at.encode_into(buf);
        encode::unsigned(buf, self.epoch);
    }
}

impl DeviceRecord {
    /// Create and sign a device record. Called by the identity holder.
    pub fn create(
        principal_seed: &[u8; 32],
        principal: IdentityRoot,
        principal_epoch: RotationEpoch,
        device_key: DeviceSigningKey,
        device_id: DeviceId,
        device_name: String,
        issued_at: Hlc,
        epoch: u64,
    ) -> Result<Self, IdentityError> {
        let kp = sign::keypair_from_seed(principal_seed)
            .map_err(|e| IdentityError::KeypairDerivation {
                reason: format!("device sign: {e}"),
            })?;

        let signable = DeviceSignable {
            principal_bytes: *principal.as_bytes(),
            principal_epoch: principal_epoch.0,
            device_key_bytes: device_key.0,
            device_id_bytes: device_id.0,
            device_name: &device_name,
            issued_at,
            epoch,
        };

        let sig = sign::sign_raw(&kp, &signable.signable_bytes());

        Ok(Self {
            principal,
            principal_epoch,
            device_key,
            device_id,
            device_name,
            issued_at,
            epoch,
            signature: Signature64::from_bytes(sig),
        })
    }

    /// Verify this device record: signature by the principal root.
    pub fn verify(&self) -> Result<(), IdentityError> {
        let signable = DeviceSignable {
            principal_bytes: *self.principal.as_bytes(),
            principal_epoch: self.principal_epoch.0,
            device_key_bytes: self.device_key.0,
            device_id_bytes: self.device_id.0,
            device_name: &self.device_name,
            issued_at: self.issued_at,
            epoch: self.epoch,
        };

        sign::verify_raw(
            self.principal.as_bytes(),
            &signable.signable_bytes(),
            self.signature.as_bytes(),
        ).map_err(|_| IdentityError::BadSignature {
            domain: derivation_tags::DEVICE_SIGN,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::originate_from_seed;
    use crate::origin::seed::OriginSeed;
    use zeroize::Zeroizing;

    fn test_identity() -> (IdentityRoot, [u8; 32]) {
        let seed = [0x01u8; 32];
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed))
        ).unwrap();
        (o.root, seed)
    }

    #[test]
    fn device_record_sign_verify() {
        let (principal, seed) = test_identity();
        let device_seed = [0xDD; 32];
        let device_kp = sign::keypair_from_seed(&device_seed).unwrap();
        let device_key = DeviceSigningKey(sign::public_key_bytes(&device_kp));
        let device_id = DeviceId::generate();

        let record = DeviceRecord::create(
            &seed, principal, RotationEpoch::ORIGIN,
            device_key, device_id, "laptop".into(),
            Hlc::now(), 0,
        ).unwrap();

        assert!(record.verify().is_ok());
    }

    #[test]
    fn device_record_wrong_signer_rejected() {
        let (principal, _seed) = test_identity();
        let wrong_seed = [0xFF; 32];
        let device_key = DeviceSigningKey([0xBB; 32]);
        let device_id = DeviceId::generate();

        let record = DeviceRecord::create(
            &wrong_seed, principal, RotationEpoch::ORIGIN,
            device_key, device_id, "phone".into(),
            Hlc::now(), 0,
        ).unwrap();

        assert!(record.verify().is_err());
    }

    #[test]
    fn device_record_tampered_name_rejected() {
        let (principal, seed) = test_identity();
        let device_key = DeviceSigningKey([0xCC; 32]);
        let device_id = DeviceId::generate();

        let mut record = DeviceRecord::create(
            &seed, principal, RotationEpoch::ORIGIN,
            device_key, device_id, "laptop".into(),
            Hlc::now(), 0,
        ).unwrap();

        record.device_name = "tampered".into();
        assert!(record.verify().is_err());
    }

    #[test]
    fn device_record_self_sign_rejected() {
        let (principal, _seed) = test_identity();
        let device_seed = [0xDD; 32];
        let device_kp = sign::keypair_from_seed(&device_seed).unwrap();
        let device_key = DeviceSigningKey(sign::public_key_bytes(&device_kp));
        let device_id = DeviceId::generate();

        // Sign with device key instead of principal
        let record = DeviceRecord::create(
            &device_seed, principal, RotationEpoch::ORIGIN,
            device_key, device_id, "laptop".into(),
            Hlc::now(), 0,
        ).unwrap();

        // verify checks against principal root, not device key
        assert!(record.verify().is_err());
    }
}
