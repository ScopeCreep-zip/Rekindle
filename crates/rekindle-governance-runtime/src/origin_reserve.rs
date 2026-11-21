//! The creator-set round's runtime half (plan E3.2 S7s.3, decision (16)):
//! a community's records are reserved before its genesis, so the anchor
//! key exists for co-creators to derive their pseudonyms for, and the
//! creator collects their signed acceptances before
//! [`finish_community`](crate::origin::finish_community) writes a cert
//! that lists them.

use rekindle_governance::genesis::verify_creator_accept;
use rekindle_secrets::derive;
use rekindle_secrets::keys::SlotSeed;
use rekindle_types::dht_layout::SLOTS_PER_SEGMENT;
use rekindle_types::id::PseudonymKey;

use crate::deps::{DhtRecordInfo, GovernanceRuntimeDeps};
use crate::error::GovernanceRuntimeError;
use crate::origin::{create_records, timed_out, OriginRecords, CREATE_DEADLINE};

/// A co-creator's verified acceptance (plan E3.2 S7s.3, decision (16)):
/// its pseudonym for the new anchor and that pseudonym's signature over
/// [`creator_accept_signing_bytes`](rekindle_governance::genesis::creator_accept_signing_bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatorAccept {
    pub pseudonym: PseudonymKey,
    pub signature: [u8; 64],
}

/// A community's records, created before its genesis (plan E3.2 S7s.3):
/// the anchor key is known, so co-creators can derive their pseudonyms
/// for it while the creator collects their acceptances. `A`'s one-time
/// owner key lives only here, in memory, until [`finish_community`](crate::origin::finish_community)
/// signs the cert with it or [`release_reserved`] drops it.
pub struct ReservedCommunity {
    pub(crate) slot_seed: SlotSeed,
    pub(crate) records: OriginRecords,
}

impl ReservedCommunity {
    /// The new anchor's record key, the community id.
    #[must_use]
    pub fn anchor_key(&self) -> &str {
        &self.records.anchor.record_key
    }
}

/// Create the community's records (`A`, `M(p)`, the registry) within
/// [`CREATE_DEADLINE`]; a failure or the deadline releases what exists.
///
/// # Errors
/// A record could not be created, or the deadline passed.
pub async fn reserve_community<D: GovernanceRuntimeDeps>(
    deps: &D,
) -> Result<ReservedCommunity, GovernanceRuntimeError> {
    let deadline = tokio::time::Instant::now() + CREATE_DEADLINE;
    let slot_seed = SlotSeed::generate_with(deps.rng().as_ref());
    let mut member_pubkeys = Vec::with_capacity(SLOTS_PER_SEGMENT as usize);
    for i in 0..SLOTS_PER_SEGMENT {
        let sk = derive::derive_slot_keypair(&slot_seed.0, i).map_err(|e| {
            GovernanceRuntimeError::Crypto(format!(
                "slot keypair derivation failed at index {i}: {e}"
            ))
        })?;
        member_pubkeys.push(sk.verifying_key().to_bytes());
    }
    let mut created: Vec<DhtRecordInfo> = Vec::with_capacity(3);
    let made = tokio::time::timeout_at(
        deadline,
        create_records(deps, &member_pubkeys, &mut created),
    )
    .await
    .unwrap_or_else(|_| Err(timed_out()));
    if let Err(e) = made {
        for record in created {
            deps.release_record(record.lease).await;
        }
        return Err(e);
    }
    let mut it = created.into_iter();
    let (Some(anchor), Some(member), Some(registry)) = (it.next(), it.next(), it.next()) else {
        return Err(GovernanceRuntimeError::Adapter(
            "origin: a record is missing".into(),
        ));
    };
    Ok(ReservedCommunity {
        slot_seed,
        records: OriginRecords {
            anchor,
            member,
            registry,
        },
    })
}

/// Give up a reserved community (the creator cancelled): every record is
/// released and `A`'s owner key dropped, so it can never get a genesis.
pub async fn release_reserved<D: GovernanceRuntimeDeps>(deps: &D, reserved: ReservedCommunity) {
    let OriginRecords {
        anchor,
        member,
        registry,
    } = reserved.records;
    for record in [anchor, member, registry] {
        deps.release_record(record.lease).await;
    }
}

/// The further creators of `anchor`'s cert: every acceptance's signature
/// verified, in the order given.
pub(crate) fn verified_creators(
    anchor: &str,
    accepted: &[CreatorAccept],
) -> Result<Vec<PseudonymKey>, GovernanceRuntimeError> {
    accepted
        .iter()
        .map(|a| {
            if verify_creator_accept(anchor, &a.pseudonym, &a.signature) {
                Ok(a.pseudonym.clone())
            } else {
                Err(GovernanceRuntimeError::Crypto(format!(
                    "co-creator {} did not sign its acceptance of {anchor}",
                    hex::encode(a.pseudonym.0)
                )))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use rekindle_governance::genesis::creator_accept_signing_bytes;
    use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};

    use super::*;

    fn accept(seed: u8, anchor: &str) -> CreatorAccept {
        let key = SigningKey::from_bytes(&[seed; 32]);
        let pseudonym = PseudonymKey(key.verifying_key().to_bytes());
        let signature = key
            .sign(&creator_accept_signing_bytes(anchor, &pseudonym))
            .to_bytes();
        CreatorAccept {
            pseudonym,
            signature,
        }
    }

    /// Decision (16): the cert lists exactly the verified acceptances, in
    /// order; one signed for another anchor refuses the whole genesis.
    #[test]
    fn only_verified_acceptances_become_further_creators() {
        let (b, c) = (accept(2, "VLD0:a"), accept(3, "VLD0:a"));
        assert_eq!(
            verified_creators("VLD0:a", &[b.clone(), c.clone()]).unwrap(),
            vec![b.pseudonym.clone(), c.pseudonym.clone()]
        );
        let elsewhere = accept(4, "VLD0:other");
        assert!(matches!(
            verified_creators("VLD0:a", &[b, elsewhere]),
            Err(GovernanceRuntimeError::Crypto(_))
        ));
    }
}
