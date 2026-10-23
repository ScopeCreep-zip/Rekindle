//! Reading the pinned `GenesisCert` from the anchor (plan E3.2, ADR 0011).
//! The anchor's descriptor owner (`owner_of`) must equal the cert's `K_A`,
//! and both signatures must verify; anything else is refused.

use rekindle_codec::governance_op::genesis::decode_genesis;
use rekindle_crypto::keypair_text::public_key_text;
use rekindle_governance::genesis::verify_genesis;
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
    let lease = deps.acquire_record(anchor, None).await?;
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
