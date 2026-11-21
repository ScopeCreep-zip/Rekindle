//! The `GenesisCert`'s signatures (plan E3.2, ADR 0011): the anchor owner
//! `K_A` and the creator pseudonym each sign
//! `GOV_GENESIS ‖ anchor ‖ unsigned`, where `unsigned` is the cert's
//! canonical encoding without signatures
//! (`rekindle_codec::governance_op::genesis::genesis_unsigned_bytes`).

use rekindle_secrets::sign::verify_signature;
use rekindle_types::domains;
use rekindle_types::governance::{GenesisCert, MAX_ADDITIONAL_CREATORS};
use rekindle_types::id::PseudonymKey;

/// Why a cert was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenesisRejection {
    /// `K_A`'s signature does not verify.
    BadAnchorSignature,
    /// The creator pseudonym's signature does not verify.
    BadCreatorSignature,
    /// More than [`MAX_ADDITIONAL_CREATORS`] further creators.
    TooManyCreators,
    /// A further creator is repeated or is the primary creator.
    RepeatedCreator,
}

/// Refuse a cert whose creator set (Q3, ADR 0011) is malformed: more than
/// [`MAX_ADDITIONAL_CREATORS`] further creators, one named twice, or the
/// primary creator named again. Checked before the cert is trusted.
///
/// # Errors
/// The creator set is malformed.
pub fn check_creator_set(cert: &GenesisCert) -> Result<(), GenesisRejection> {
    if cert.additional_creators.len() > MAX_ADDITIONAL_CREATORS {
        return Err(GenesisRejection::TooManyCreators);
    }
    let mut seen = std::collections::BTreeSet::from([&cert.creator_pseudonym.0]);
    if cert
        .additional_creators
        .iter()
        .all(|key| seen.insert(&key.0))
    {
        Ok(())
    } else {
        Err(GenesisRejection::RepeatedCreator)
    }
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

/// The bytes a co-creator's pseudonym signs to accept a place in the
/// creator set of `anchor` (plan E3.2 S7s.3, decision (16)):
/// `GOV_CREATOR_ACCEPT ‖ anchor ‖ pseudonym`.
#[must_use]
pub fn creator_accept_signing_bytes(anchor: &str, pseudonym: &PseudonymKey) -> Vec<u8> {
    let label = domains::GOV_CREATOR_ACCEPT.as_bytes();
    let mut out = Vec::with_capacity(label.len() + anchor.len() + pseudonym.0.len());
    out.extend_from_slice(label);
    out.extend_from_slice(anchor.as_bytes());
    out.extend_from_slice(&pseudonym.0);
    out
}

/// Whether `signature` is `pseudonym`'s acceptance of a place in the
/// creator set of `anchor`.
#[must_use]
pub fn verify_creator_accept(anchor: &str, pseudonym: &PseudonymKey, signature: &[u8; 64]) -> bool {
    verify_signature(
        &pseudonym.0,
        &creator_accept_signing_bytes(anchor, pseudonym),
        signature,
    )
    .is_ok()
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

    fn cert_with(additional: &[u8]) -> GenesisCert {
        let mut cert = crate::fixture::cert(
            &PseudonymKey([1; 32]),
            rekindle_types::governance::AdmissionMode::Open,
        );
        cert.additional_creators = additional.iter().map(|b| PseudonymKey([*b; 32])).collect();
        cert
    }

    #[test]
    fn a_creator_set_with_a_repeat_or_a_ninth_is_refused() {
        assert_eq!(check_creator_set(&cert_with(&[2, 3])), Ok(()));
        assert_eq!(
            check_creator_set(&cert_with(&[2, 3, 4, 5, 6, 7, 8, 9])),
            Ok(())
        );
        assert_eq!(
            check_creator_set(&cert_with(&[2, 3, 4, 5, 6, 7, 8, 9, 10])),
            Err(GenesisRejection::TooManyCreators)
        );
        assert_eq!(
            check_creator_set(&cert_with(&[2, 3, 2])),
            Err(GenesisRejection::RepeatedCreator)
        );
        assert_eq!(
            check_creator_set(&cert_with(&[2, 1])),
            Err(GenesisRejection::RepeatedCreator)
        );
    }

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

    /// Decision (16): an acceptance verifies only for its own anchor and
    /// pseudonym, under the creator-accept label.
    #[test]
    fn a_creator_accept_binds_its_anchor_and_pseudonym() {
        let key = SigningKey::from_bytes(&[9; 32]);
        let pseudo = PseudonymKey(key.verifying_key().to_bytes());
        let sig = key
            .sign(&creator_accept_signing_bytes("VLD0:a", &pseudo))
            .to_bytes();
        assert!(verify_creator_accept("VLD0:a", &pseudo, &sig));
        assert!(!verify_creator_accept("VLD0:b", &pseudo, &sig));
        let other = PseudonymKey(SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes());
        assert!(!verify_creator_accept("VLD0:a", &other, &sig));
        let genesis_sig = key
            .sign(&genesis_signing_bytes("VLD0:a", &pseudo.0))
            .to_bytes();
        assert!(!verify_creator_accept("VLD0:a", &pseudo, &genesis_sig));
    }
}
