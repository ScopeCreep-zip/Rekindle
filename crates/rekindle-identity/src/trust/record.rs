//! TrustRecord — per-peer trust state with the PreviouslyVerified latch.
//!
//! One record per known peer, keyed by `PeerRef`. Persisted to the vault
//! via `TrustStore::export()` / `import()`. The latch is the most
//! security-critical field: it persists across restarts, rotations,
//! and withdrawal — it is NEVER cleared except by identity death.

use crate::origin::originate::{IdentityRoot, RotationEpoch};
use crate::wire::signable::Hlc;

use super::state::TrustState;

/// Per-peer trust record.
///
/// `anchor_root` is the stable origination-epoch root (the map key
/// material). `pinned_root` is the current chain head (advances on
/// rotation). The `TrustStore` reconstructs `PeerRef` from
/// `anchor_root` at import — `PeerRef` is a runtime handle, not a
/// persisted field.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrustRecord {
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
    /// The PreviouslyVerified latch. Set on `Verified`, NEVER cleared.
    pub previously_verified: bool,
    /// Active rotation grace window end time. `None` if no grace is active.
    /// Voided immediately by a `RevocationCertificate`.
    pub grace_until: Option<Hlc>,
    /// The epoch at which a `RevocationCertificate` was issued, if revoked.
    /// `None` if not revoked. Used to distinguish "revoked at origination"
    /// from "revoked after rotation" — material signed between a rotation
    /// and a revocation has different trust semantics.
    pub revoked_at_epoch: Option<RotationEpoch>,
    /// Number of rotation proof links observed (for diagnostic display).
    pub rotation_chain_length: u64,
}

impl TrustRecord {
    /// Create a new record for first contact with a peer.
    /// The root becomes both the anchor and the initial pinned head.
    pub fn first_contact(root: IdentityRoot) -> Self {
        Self {
            anchor_root: root,
            pinned_root: root,
            pinned_epoch: RotationEpoch::ORIGIN,
            state: TrustState::Pinned,
            previously_verified: false,
            grace_until: None,
            revoked_at_epoch: None,
            rotation_chain_length: 0,
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
    pub fn advance_rotation(
        &mut self,
        new_root: IdentityRoot,
        new_epoch: RotationEpoch,
        grace_until: Hlc,
        chain_length: u64,
    ) {
        self.pinned_root = new_root;
        self.pinned_epoch = new_epoch;
        self.grace_until = Some(grace_until);
        self.rotation_chain_length += chain_length;
    }
}

/// Serializable projection for vault persistence.
///
/// Same as `TrustRecord` but with `serde` attributes for JSON storage.
/// The `TrustStore` converts between these on export/import.
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
    }

    #[test]
    fn grace_window_check() {
        let mut record = TrustRecord::first_contact(test_root());
        let now = Hlc::now();

        assert!(!record.is_within_grace(&now));

        // Set grace to far future
        let far_future = Hlc::new(now.physical_ns + 999_000_000_000, 0);
        record.grace_until = Some(far_future);
        assert!(record.is_within_grace(&now));

        // Set grace to past
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
        let new_root = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x02; 32]))
        ).unwrap().root;

        let grace = Hlc::new(999_999_999, 0);
        record.advance_rotation(new_root, RotationEpoch(1), grace, 1);

        assert_eq!(record.pinned_root, new_root);
        assert_eq!(record.pinned_epoch, RotationEpoch(1));
        assert_eq!(record.grace_until, Some(grace));
        assert_eq!(record.rotation_chain_length, 1);
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
    }
}
