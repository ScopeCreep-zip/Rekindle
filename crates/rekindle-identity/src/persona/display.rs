//! DisplayName — a signed, mutable, non-unique human-readable label.
//!
//! Never a lookup key, never part of equality. Signed by the
//! `IdentityRoot` so recipients can verify the claim.

use rekindle_ratchet::crypto::sign;

use crate::error::IdentityError;
use crate::origin::originate::IdentityRoot;
use crate::origin::tags::derivation_tags;
use crate::wire::encode;
use crate::wire::signable::{Hlc, Signable, Signature64};

/// A signed display name claim.
///
/// The name itself is untrusted user input — it may be empty, duplicate,
/// or misleading. The signature proves only that the holder of the
/// `IdentityRoot` private key asserted this name at `issued_at`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DisplayName {
    pub name: String,
    pub root: IdentityRoot,
    pub issued_at: Hlc,
    pub signature: Signature64,
}

struct DisplayNameSignable<'a> {
    name: &'a str,
    root_bytes: [u8; 32],
    issued_at: Hlc,
}

impl Signable for DisplayNameSignable<'_> {
    const SIGN_DOMAIN: &'static str = derivation_tags::DISPLAY_NAME_SIGN;

    fn encode_fields(&self, buf: &mut Vec<u8>) {
        // WIRE FORMAT: field order pinned — reordering is a wire break
        encode::array_head(buf, 3);
        encode::text(buf, self.name);
        encode::bytes(buf, &self.root_bytes);
        self.issued_at.encode_into(buf);
    }
}

impl DisplayName {
    /// Create and sign a display name claim.
    pub fn create(
        seed: &[u8; 32],
        root: IdentityRoot,
        name: String,
        issued_at: Hlc,
    ) -> Result<Self, IdentityError> {
        let kp = sign::keypair_from_seed(seed)
            .map_err(|e| IdentityError::KeypairDerivation {
                reason: format!("display name sign: {e}"),
            })?;

        let signable = DisplayNameSignable {
            name: &name,
            root_bytes: *root.as_bytes(),
            issued_at,
        };

        let sig = sign::sign_raw(&kp, &signable.signable_bytes());

        Ok(Self {
            name,
            root,
            issued_at,
            signature: Signature64::from_bytes(sig),
        })
    }

    /// Verify the display name signature.
    pub fn verify(&self) -> Result<(), IdentityError> {
        let signable = DisplayNameSignable {
            name: &self.name,
            root_bytes: *self.root.as_bytes(),
            issued_at: self.issued_at,
        };

        sign::verify_raw(
            self.root.as_bytes(),
            &signable.signable_bytes(),
            self.signature.as_bytes(),
        ).map_err(|_| IdentityError::BadSignature {
            domain: derivation_tags::DISPLAY_NAME_SIGN,
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
    fn create_and_verify() {
        let (root, seed) = test_identity();
        let dn = DisplayName::create(&seed, root, "alice".into(), Hlc::now()).unwrap();
        assert!(dn.verify().is_ok());
        assert_eq!(dn.name, "alice");
    }

    #[test]
    fn tampered_name_rejected() {
        let (root, seed) = test_identity();
        let mut dn = DisplayName::create(&seed, root, "alice".into(), Hlc::now()).unwrap();
        dn.name = "eve".into();
        assert!(dn.verify().is_err());
    }

    #[test]
    fn wrong_signer_rejected() {
        let (root, _seed) = test_identity();
        let wrong_seed = [0xFF; 32];
        let dn = DisplayName::create(&wrong_seed, root, "alice".into(), Hlc::now()).unwrap();
        assert!(dn.verify().is_err());
    }

    #[test]
    fn empty_name_valid() {
        let (root, seed) = test_identity();
        let dn = DisplayName::create(&seed, root, String::new(), Hlc::now()).unwrap();
        assert!(dn.verify().is_ok());
    }
}
