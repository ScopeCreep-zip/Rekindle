//! The `GenesisCert`'s signatures (plan E3.2, ADR 0011): the anchor owner
//! `K_A` and the creator pseudonym each sign
//! `GOV_GENESIS ‖ anchor ‖ unsigned`, where `unsigned` is the cert's
//! canonical encoding without signatures
//! (`rekindle_codec::governance_op::genesis::genesis_unsigned_bytes`).

use rekindle_secrets::sign::verify_signature;
use rekindle_types::domains;
use rekindle_types::id::PseudonymKey;

/// Why a cert was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenesisRejection {
    /// `K_A`'s signature does not verify.
    BadAnchorSignature,
    /// The creator pseudonym's signature does not verify.
    BadCreatorSignature,
}

/// The bytes both genesis signers sign: `GOV_GENESIS ‖ anchor ‖ unsigned`.
#[must_use]
pub fn genesis_signing_bytes(anchor: &str, unsigned: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(domains::GOV_GENESIS.len() + anchor.len() + unsigned.len());
    out.extend_from_slice(domains::GOV_GENESIS.as_bytes());
    out.extend_from_slice(anchor.as_bytes());
    out.extend_from_slice(unsigned);
    out
}

/// Verify both signatures of the cert of community `anchor`. The caller
/// pins `anchor_owner` to the anchor record's descriptor owner
/// (`RecordPort::owner_of`) before trusting the cert.
///
/// # Errors
/// Either signature fails.
pub fn verify_genesis(
    anchor: &str,
    unsigned: &[u8],
    anchor_owner: &[u8; 32],
    anchor_sig: &[u8; 64],
    creator: &PseudonymKey,
    creator_sig: &[u8; 64],
) -> Result<(), GenesisRejection> {
    let message = genesis_signing_bytes(anchor, unsigned);
    verify_signature(anchor_owner, &message, anchor_sig)
        .map_err(|_| GenesisRejection::BadAnchorSignature)?;
    verify_signature(&creator.0, &message, creator_sig)
        .map_err(|_| GenesisRejection::BadCreatorSignature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn both_signers_must_sign_the_same_bytes() {
        let k_a = SigningKey::from_bytes(&[7; 32]);
        let creator = SigningKey::from_bytes(&[9; 32]);
        let msg = genesis_signing_bytes("VLD0:a", b"cert");
        let a_sig = k_a.sign(&msg).to_bytes();
        let c_sig = creator.sign(&msg).to_bytes();
        let a_pub = k_a.verifying_key().to_bytes();
        let c_pub = PseudonymKey(creator.verifying_key().to_bytes());
        assert_eq!(
            verify_genesis("VLD0:a", b"cert", &a_pub, &a_sig, &c_pub, &c_sig),
            Ok(())
        );
        assert_eq!(
            verify_genesis("VLD0:b", b"cert", &a_pub, &a_sig, &c_pub, &c_sig),
            Err(GenesisRejection::BadAnchorSignature)
        );
        assert_eq!(
            verify_genesis("VLD0:a", b"cert", &a_pub, &a_sig, &c_pub, &a_sig),
            Err(GenesisRejection::BadCreatorSignature)
        );
    }
}
