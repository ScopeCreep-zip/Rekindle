//! Signed, versioned locator record — the set of network addresses
//! a peer publishes for discovery.
//!
//! `LocatorRecord` carries a `locator_epoch` (monotonic per root chain,
//! latest-wins) so staleness between two cached copies is detectable
//! and orderable. The record is signed by the `IdentityRoot`.

use rekindle_ratchet::crypto::sign;

use crate::error::IdentityError;
use crate::origin::originate::{IdentityRoot, RotationEpoch};
use crate::origin::tags::derivation_tags;
use crate::wire::encode;
use crate::wire::signable::{Hlc, Signable, Signature64};

/// The kind of locator in a record entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum LocatorKind {
    Profile,
    Mailbox,
    Inbox,
}

impl LocatorKind {
    fn discriminant(self) -> u64 {
        match self {
            Self::Profile => 0,
            Self::Mailbox => 1,
            Self::Inbox => 2,
        }
    }
}

/// A single entry in a locator record: kind + raw locator display form.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocatorEntry {
    pub kind: LocatorKind,
    /// The full display form (e.g., "VLD0:abc123").
    pub display_form: String,
}

/// The signed, versioned locator set a peer publishes.
#[derive(Debug, Clone)]
pub struct LocatorRecord {
    pub root: IdentityRoot,
    pub epoch: RotationEpoch,
    /// Monotonic per root-chain. Latest wins.
    pub locator_epoch: u64,
    pub entries: Vec<LocatorEntry>,
    pub issued_at: Hlc,
    pub signature: Signature64,
}

struct LocatorSignable<'a> {
    root_bytes: [u8; 32],
    epoch: u64,
    locator_epoch: u64,
    entries: &'a [LocatorEntry],
    issued_at: Hlc,
}

impl Signable for LocatorSignable<'_> {
    const SIGN_DOMAIN: &'static str = derivation_tags::LOCATOR_SIGN;

    fn encode_fields(&self, buf: &mut Vec<u8>) {
        // WIRE FORMAT: field order pinned — reordering is a wire break
        encode::array_head(buf, 5);
        encode::bytes(buf, &self.root_bytes);
        encode::unsigned(buf, self.epoch);
        encode::unsigned(buf, self.locator_epoch);
        // Entries as a nested array of 2-element arrays [kind_discriminant, display_form]
        encode::array_head(buf, self.entries.len() as u64);
        for entry in self.entries {
            encode::array_head(buf, 2);
            encode::unsigned(buf, entry.kind.discriminant());
            encode::text(buf, &entry.display_form);
        }
        self.issued_at.encode_into(buf);
    }
}

impl LocatorRecord {
    /// Create and sign a locator record.
    pub fn create(
        seed: &[u8; 32],
        root: IdentityRoot,
        epoch: RotationEpoch,
        locator_epoch: u64,
        entries: Vec<LocatorEntry>,
        issued_at: Hlc,
    ) -> Result<Self, IdentityError> {
        let kp = sign::keypair_from_seed(seed)
            .map_err(|e| IdentityError::KeypairDerivation {
                reason: format!("locator sign: {e}"),
            })?;

        let signable = LocatorSignable {
            root_bytes: *root.as_bytes(),
            epoch: epoch.0,
            locator_epoch,
            entries: &entries,
            issued_at,
        };

        let sig = sign::sign_raw(&kp, &signable.signable_bytes());

        Ok(Self {
            root,
            epoch,
            locator_epoch,
            entries,
            issued_at,
            signature: Signature64::from_bytes(sig),
        })
    }

    /// Verify the signature.
    pub fn verify(&self) -> Result<(), IdentityError> {
        let signable = LocatorSignable {
            root_bytes: *self.root.as_bytes(),
            epoch: self.epoch.0,
            locator_epoch: self.locator_epoch,
            entries: &self.entries,
            issued_at: self.issued_at,
        };

        sign::verify_raw(
            self.root.as_bytes(),
            &signable.signable_bytes(),
            self.signature.as_bytes(),
        ).map_err(|_| IdentityError::BadSignature {
            domain: derivation_tags::LOCATOR_SIGN,
        })
    }

    /// Check that this record's locator_epoch is not stale relative
    /// to the locally known epoch for this peer.
    pub fn check_freshness(&self, known_locator_epoch: u64) -> Result<(), IdentityError> {
        if self.locator_epoch < known_locator_epoch {
            return Err(IdentityError::StaleLocator {
                presented: self.locator_epoch,
                known: known_locator_epoch,
            });
        }
        Ok(())
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
        let entries = vec![
            LocatorEntry { kind: LocatorKind::Profile, display_form: "VLD0:prof123".into() },
            LocatorEntry { kind: LocatorKind::Inbox, display_form: "VLD0:inbox456".into() },
        ];

        let record = LocatorRecord::create(
            &seed, root, RotationEpoch::ORIGIN, 1, entries, Hlc::now(),
        ).unwrap();

        assert!(record.verify().is_ok());
        assert_eq!(record.entries.len(), 2);
    }

    #[test]
    fn tampered_entry_rejected() {
        let (root, seed) = test_identity();
        let entries = vec![
            LocatorEntry { kind: LocatorKind::Profile, display_form: "VLD0:orig".into() },
        ];

        let mut record = LocatorRecord::create(
            &seed, root, RotationEpoch::ORIGIN, 1, entries, Hlc::now(),
        ).unwrap();

        record.entries[0].display_form = "VLD0:tampered".into();
        assert!(record.verify().is_err());
    }

    #[test]
    fn stale_locator_epoch_rejected() {
        let (root, seed) = test_identity();
        let record = LocatorRecord::create(
            &seed, root, RotationEpoch::ORIGIN, 3, vec![], Hlc::now(),
        ).unwrap();

        assert!(record.check_freshness(4).is_err());
        assert!(record.check_freshness(3).is_ok());
        assert!(record.check_freshness(2).is_ok());
    }

    #[test]
    fn empty_entries_valid() {
        let (root, seed) = test_identity();
        let record = LocatorRecord::create(
            &seed, root, RotationEpoch::ORIGIN, 0, vec![], Hlc::now(),
        ).unwrap();
        assert!(record.verify().is_ok());
    }
}
