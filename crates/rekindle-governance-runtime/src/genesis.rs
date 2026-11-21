//! Reading the pinned `GenesisCert` from the anchor (plan E3.2, ADR 0011).
//! The anchor's descriptor owner (`owner_of`) must equal the cert's `K_A`,
//! its creator set well formed (Q3) and both signatures must verify;
//! anything else is refused.

use rekindle_codec::governance_op::genesis::decode_genesis;
use rekindle_crypto::keypair_text::public_key_text;
use rekindle_governance::genesis::{check_creator_set, verify_genesis};
use rekindle_records::port::OpenIntent;
use rekindle_types::dht_layout::community_anchor;
use rekindle_types::governance::GenesisCert;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;

/// Read `anchor`/0 from the network, pin it to the descriptor owner and
/// verify it. Returns the cert and its bytes (to store with the row).
///
/// # Errors
/// The anchor is unreadable or empty, the cert does not decode, its `K_A`
/// is not the anchor's owner, or a signature fails.
pub async fn read_pinned<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
) -> Result<(GenesisCert, Vec<u8>), GovernanceRuntimeError> {
    let lease = deps.acquire_record(anchor, None, OpenIntent::Check).await?;
    let read = deps
        .get_dht_value(lease, community_anchor::GENESIS_CERT, true)
        .await;
    let owner = deps.owner_of(lease);
    deps.release_record(lease).await;
    let bytes = read?.ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(anchor.into()))?;
    let owner = owner?;
    let decoded =
        decode_genesis(&bytes).map_err(|e| GovernanceRuntimeError::Encoding(e.to_string()))?;
    let signed = decoded.signed;
    if public_key_text(&signed.anchor_owner) != owner {
        return Err(GovernanceRuntimeError::Crypto(format!(
            "genesis cert of {anchor} is not signed by the anchor's owner"
        )));
    }
    check_creator_set(&signed.cert)
        .map_err(|e| GovernanceRuntimeError::Crypto(format!("genesis cert of {anchor}: {e:?}")))?;
    verify_genesis(
        anchor,
        &decoded.unsigned,
        &signed.anchor_owner,
        &signed.anchor_sig,
        &signed.cert.creator_pseudonym,
        &signed.creator_sig,
    )
    .map_err(|e| GovernanceRuntimeError::Crypto(format!("genesis cert of {anchor}: {e:?}")))?;
    Ok((signed.cert, bytes))
}

#[cfg(test)]
mod tests {
    use rekindle_codec::governance_op::genesis::{
        decode_genesis, encode_genesis, genesis_unsigned_bytes, SignedGenesis,
    };
    use rekindle_governance::genesis::{genesis_signing_bytes, verify_genesis, GenesisRejection};
    use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
    use rekindle_types::governance::{AdmissionMode, GenesisCert};
    use rekindle_types::id::PseudonymKey;

    const ANCHOR: &str = "VLD0:anchor";

    /// A cert with two further creators and a predecessor, signed by both.
    fn signed() -> (SignedGenesis, SigningKey) {
        let k_a = SigningKey::from_bytes(&[7; 32]);
        let creator = SigningKey::from_bytes(&[9; 32]);
        let mut cert: GenesisCert = rekindle_governance::fixture::cert(
            &PseudonymKey(creator.verifying_key().to_bytes()),
            AdmissionMode::Open,
        );
        cert.additional_creators = vec![PseudonymKey([2; 32]), PseudonymKey([3; 32])];
        cert.predecessor = Some("VLD0:old".into());
        let anchor_owner = k_a.verifying_key().to_bytes();
        let message = genesis_signing_bytes(ANCHOR, &genesis_unsigned_bytes(&cert, &anchor_owner));
        let signed = SignedGenesis {
            anchor_sig: k_a.sign(&message).to_bytes(),
            creator_sig: creator.sign(&message).to_bytes(),
            cert,
            anchor_owner,
        };
        (signed, creator)
    }

    /// Re-encode `signed` (sigs kept), decode it as a reader would, verify.
    fn verify_as_read(signed: &SignedGenesis) -> Result<(), GenesisRejection> {
        let decoded = decode_genesis(&encode_genesis(signed).unwrap()).unwrap();
        let s = decoded.signed;
        verify_genesis(
            ANCHOR,
            &decoded.unsigned,
            &s.anchor_owner,
            &s.anchor_sig,
            &s.cert.creator_pseudonym,
            &s.creator_sig,
        )
    }

    #[test]
    fn a_tampered_creator_set_or_predecessor_fails_the_cert_signature() {
        let (signed, _) = signed();
        assert_eq!(verify_as_read(&signed), Ok(()));
        let mut added = signed.clone();
        added.cert.additional_creators.push(PseudonymKey([4; 32]));
        assert_eq!(
            verify_as_read(&added),
            Err(GenesisRejection::BadAnchorSignature)
        );
        let mut dropped = signed.clone();
        dropped.cert.additional_creators.remove(0);
        assert_eq!(
            verify_as_read(&dropped),
            Err(GenesisRejection::BadAnchorSignature)
        );
        let mut moved = signed.clone();
        moved.cert.predecessor = Some("VLD0:other".into());
        assert_eq!(
            verify_as_read(&moved),
            Err(GenesisRejection::BadAnchorSignature)
        );
        let mut unset = signed;
        unset.cert.predecessor = None;
        assert_eq!(
            verify_as_read(&unset),
            Err(GenesisRejection::BadAnchorSignature)
        );
    }
}
