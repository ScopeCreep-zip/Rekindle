//! TrustRecord — per-peer trust state with the PreviouslyVerified latch.
//!
//! One record per known peer, keyed by `PeerRef`. Persisted to the vault
//! via `TrustStore::export()` / `import()`.
//!
//! **Versioned persistence:** `TrustRecordPersist` carries a `version`
//! discriminant. Deserialization always upgrades to the latest version.
//! Adding a field requires a version bump and migration logic.
//! Per matrix-rust-sdk `OtherUserIdentityDataSerializer` pattern.

use crate::origin::originate::{IdentityRoot, RotationEpoch};
use crate::wire::signable::Hlc;

use super::state::TrustState;

/// Persistence format version. Bump on any field addition.
const CURRENT_VERSION: u8 = 1;

/// Per-peer trust record.
///
/// `anchor_root` is the stable origination-epoch root (the map key
/// material). `pinned_root` is the current chain head (advances on
/// rotation). The `TrustStore` reconstructs `PeerRef` from
/// `anchor_root` at import — `PeerRef` is a runtime handle, not a
/// persisted field.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrustRecord {
    /// Persistence version. Always serialized at CURRENT_VERSION.
    /// Deserialization migrates older versions on read.
    #[serde(default = "default_version")]
    pub version: u8,
    /// The stable anchor root — origination-epoch or first-observed.
    /// This is what the `TrustStore` DashMap keys by (via `PeerRef`).
    /// Does NOT change on rotation.
    pub anchor_root: IdentityRoot,
    /// The current chain head — advances on rotation. Used for
    /// signature verification of inbound material from this peer.
    pub pinned_root: IdentityRoot,
    /// The highest rotation epoch observed for this peer.
    pub pinned_epoch: RotationEpoch,
    /// Current trust state.
    pub state: TrustState,
    /// The PreviouslyVerified latch. Set on `Verified`, cleared only
    /// by explicit `withdraw_verification()`.
    pub previously_verified: bool,
    /// Active rotation grace window end time. `None` if no grace is active.
    /// Voided immediately by a `RevocationCertificate`.
    pub grace_until: Option<Hlc>,
    /// The epoch at which a `RevocationCertificate` was issued, if revoked.
    pub revoked_at_epoch: Option<RotationEpoch>,
    /// Number of rotation proof links observed (for diagnostic display).
    pub rotation_chain_length: u64,
    /// The immediately-prior head root. Used by `TrustStore::import()` to
    /// rebuild the `prior_head` index. `None` at first contact (no prior head).
    #[serde(default)]
    pub prior_head_root: Option<IdentityRoot>,
    /// When this peer was first observed. Used for diagnostics and
    /// future migration gates. Defaults to epoch 0 for V0 records.
    #[serde(default = "default_first_contact")]
    pub first_contact_at: Hlc,
}

fn default_version() -> u8 { CURRENT_VERSION }
fn default_first_contact() -> Hlc { Hlc::new(0, 0) }

impl TrustRecord {
    /// Create a new record for first contact with a peer.
    /// The root becomes both the anchor and the initial pinned head.
    pub fn first_contact(root: IdentityRoot) -> Self {
        Self {
            version: CURRENT_VERSION,
            anchor_root: root,
            pinned_root: root,
            pinned_epoch: RotationEpoch::ORIGIN,
            state: TrustState::Pinned,
            previously_verified: false,
            grace_until: None,
            revoked_at_epoch: None,
            rotation_chain_length: 0,
            prior_head_root: None,
            first_contact_at: Hlc::now(),
        }
    }

    /// Whether old-root-signed material should be accepted (within grace window).
    pub fn is_within_grace(&self, now: &Hlc) -> bool {
        self.grace_until.is_some_and(|deadline| now <= &deadline)
    }

    /// Void the grace window (called on revocation).
    pub fn void_grace(&mut self) {
        self.grace_until = None;
    }

    /// Advance the head root and epoch after a verified rotation chain.
    /// Preserves the old head as `prior_head_root` for index rebuild on import.
    pub fn advance_rotation(
        &mut self,
        new_root: IdentityRoot,
        new_epoch: RotationEpoch,
        grace_until: Hlc,
        chain_length: u64,
    ) {
        self.prior_head_root = Some(self.pinned_root);
        self.pinned_root = new_root;
        self.pinned_epoch = new_epoch;
        self.grace_until = Some(grace_until);
        self.rotation_chain_length += chain_length;
    }
}

/// Serializable projection for vault persistence.
/// Same struct — versioned via the `version` field.
pub type TrustRecordPersist = TrustRecord;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::originate_from_seed;
    use crate::origin::seed::OriginSeed;
    use zeroize::Zeroizing;

    fn test_root() -> IdentityRoot {
        originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]))
        ).unwrap().root
    }

    #[test]
    fn first_contact_is_pinned() {
        let record = TrustRecord::first_contact(test_root());
        assert_eq!(record.state, TrustState::Pinned);
        assert!(!record.previously_verified);
        assert!(record.grace_until.is_none());
        assert!(record.prior_head_root.is_none());
        assert_eq!(record.version, CURRENT_VERSION);
    }

    #[test]
    fn grace_window_check() {
        let mut record = TrustRecord::first_contact(test_root());
        let now = Hlc::now();

        assert!(!record.is_within_grace(&now));

        let far_future = Hlc::new(now.physical_ns + 999_000_000_000, 0);
        record.grace_until = Some(far_future);
        assert!(record.is_within_grace(&now));

        let past = Hlc::new(1, 0);
        record.grace_until = Some(past);
        assert!(!record.is_within_grace(&now));
    }

    #[test]
    fn void_grace_clears_window() {
        let mut record = TrustRecord::first_contact(test_root());
        record.grace_until = Some(Hlc::new(u64::MAX, 0));
        record.void_grace();
        assert!(record.grace_until.is_none());
    }

    #[test]
    fn advance_rotation_updates_fields() {
        let mut record = TrustRecord::first_contact(test_root());
        let old_root = record.pinned_root;
        let new_root = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x02; 32]))
        ).unwrap().root;

        let grace = Hlc::new(999_999_999, 0);
        record.advance_rotation(new_root, RotationEpoch(1), grace, 1);

        assert_eq!(record.pinned_root, new_root);
        assert_eq!(record.pinned_epoch, RotationEpoch(1));
        assert_eq!(record.grace_until, Some(grace));
        assert_eq!(record.rotation_chain_length, 1);
        assert_eq!(record.prior_head_root, Some(old_root),
            "prior_head_root must be set to the old pinned_root");
    }

    #[test]
    fn serde_roundtrip() {
        let record = TrustRecord::first_contact(test_root());
        let json = serde_json::to_string(&record).unwrap();
        let restored: TrustRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.anchor_root, record.anchor_root);
        assert_eq!(restored.pinned_root, record.pinned_root);
        assert_eq!(restored.state, record.state);
        assert_eq!(restored.previously_verified, record.previously_verified);
        assert_eq!(restored.version, CURRENT_VERSION);
    }

    #[test]
    fn deserialize_v0_migrates() {
        // V0 records have no version, no prior_head_root, no first_contact_at.
        // serde(default) provides safe defaults for all new fields.
        let v0_json = serde_json::json!({
            "anchor_root": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "pinned_root": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "pinned_epoch": 0,
            "state": "Pinned",
            "previously_verified": false,
            "grace_until": null,
            "revoked_at_epoch": null,
            "rotation_chain_length": 0
        });
        let record: TrustRecord = serde_json::from_value(v0_json).unwrap();
        assert_eq!(record.version, CURRENT_VERSION, "missing version field defaults to current");
        assert!(record.prior_head_root.is_none(), "missing prior_head_root defaults to None");
        assert_eq!(record.first_contact_at, Hlc::new(0, 0), "missing first_contact_at defaults to epoch 0");
    }
}
