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
#[path = "genesis_tests.rs"]
mod tests;
