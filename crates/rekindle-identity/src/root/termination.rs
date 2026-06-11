//! Death notice — voluntary identity termination.
//!
//! A `DeathNotice` is a root-signed statement declaring the identity
//! voluntarily dead. Vault destruction follows locally; peers tear
//! down sessions and enter the terminal `Dead` trust substate.

use rekindle_ratchet::crypto::sign;

use crate::error::IdentityError;
use crate::origin::originate::{IdentityRoot, RotationEpoch};
use crate::origin::tags::derivation_tags;
use crate::wire::encode;
use crate::wire::signable::{Hlc, Signable, Signature64};

/// Voluntary identity termination notice.
#[derive(Debug, Clone)]
pub struct DeathNotice {
    pub root: IdentityRoot,
    pub epoch: RotationEpoch,
    pub issued_at: Hlc,
    pub signature: Signature64,
}

struct DeathSignable {
    root_bytes: [u8; 32],
    epoch: u64,
    issued_at: Hlc,
}

impl Signable for DeathSignable {
    const SIGN_DOMAIN: &'static str = derivation_tags::DEATH_SIGN;

    fn encode_fields(&self, buf: &mut Vec<u8>) {
        // WIRE FORMAT: field order pinned — reordering is a wire break
        encode::array_head(buf, 3);
        encode::bytes(buf, &self.root_bytes);
        encode::unsigned(buf, self.epoch);
        self.issued_at.encode_into(buf);
    }
}

impl DeathNotice {
    /// Create and sign a death notice.
    pub fn create(
        seed: &[u8; 32],
        root: IdentityRoot,
        epoch: RotationEpoch,
        issued_at: Hlc,
    ) -> Result<Self, IdentityError> {
        let kp = sign::keypair_from_seed(seed)
            .map_err(|e| IdentityError::KeypairDerivation {
                reason: format!("death sign: {e}"),
            })?;
        let signable = DeathSignable {
            root_bytes: *root.as_bytes(),
            epoch: epoch.0,
            issued_at,
        };
        let sig = sign::sign_raw(&kp, &signable.signable_bytes());
        Ok(Self {
            root,
            epoch,
            issued_at,
            signature: Signature64::from_bytes(sig),
        })
    }

    /// Verify the death notice signature.
    pub fn verify(&self) -> Result<(), IdentityError> {
        let signable = DeathSignable {
            root_bytes: *self.root.as_bytes(),
            epoch: self.epoch.0,
            issued_at: self.issued_at,
        };
        sign::verify_raw(
            self.root.as_bytes(),
            &signable.signable_bytes(),
            self.signature.as_bytes(),
        ).map_err(|_| IdentityError::BadSignature {
            domain: derivation_tags::DEATH_SIGN,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::originate_from_seed;
    use crate::origin::seed::OriginSeed;
    use zeroize::Zeroizing;

    #[test]
    fn death_notice_roundtrip() {
        let seed = [0x01u8; 32];
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed))
        ).unwrap();

        let notice = DeathNotice::create(
            &seed, o.root, RotationEpoch::ORIGIN, Hlc::now(),
        ).unwrap();

        assert!(notice.verify().is_ok());
    }

    #[test]
    fn death_notice_tampered_rejected() {
        let seed = [0x01u8; 32];
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed))
        ).unwrap();

        let notice = DeathNotice::create(
            &seed, o.root, RotationEpoch::ORIGIN, Hlc::now(),
        ).unwrap();

        // Tamper with epoch
        let mut tampered = notice.clone();
        tampered.epoch = RotationEpoch(999);
        assert!(tampered.verify().is_err());
    }

    #[test]
    fn death_notice_wrong_signer_rejected() {
        let seed_a = [0x01u8; 32];
        let seed_b = [0x02u8; 32];
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed_a))
        ).unwrap();

        // Sign with wrong seed
        let notice = DeathNotice::create(
            &seed_b, o.root, RotationEpoch::ORIGIN, Hlc::now(),
        ).unwrap();

        assert!(notice.verify().is_err());
    }
}
