//! SelfIdentity — our own PeerId plus the private OriginSeed.
//!
//! This is the type that replaces `SessionIdentity` in rekindle-types
//! and `SigningKeyHandle` in rekindle-chat. Every method delegates to
//! an existing frozen derivation primitive — no new crypto here.
//!
//! Invariants:
//! - NOT `Clone` (OriginSeed is !Clone — one owner per seed)
//! - NOT `Serialize`/`Deserialize` (the seed must never be serialized
//!   without vault encryption)
//! - Debug redacts the seed
//!
//! **Own verification state:** `SelfVerificationState` is distinct from
//! the peer `TrustState`. Per matrix-rust-sdk, own identity has three
//! states (NeverVerified, Verified, VerificationViolation) and
//! `withdraw_own_verification()` only transitions
//! VerificationViolation → NeverVerified.

use zeroize::Zeroizing;

use rekindle_ratchet::crypto::sign;

use crate::error::IdentityError;
use crate::locator::GovernanceKey;
use crate::origin::originate::{
    DhSeed, IdentityRoot, OriginatedIdentity, RestoredIdentity, RotationEpoch,
};
use crate::origin::seed::OriginSeed;
use crate::origin::tags::derivation_tags;
use crate::peer::{CryptoIdentity, NetworkAddr, PeerId, SocialProfile};
use crate::projection::pseudonym::{CommunityPersona, PersonaSecrets};
use crate::session::{session_anchor, SessionAnchor};
use crate::wire::signable::Signature64;

/// Own identity verification state — distinct from peer TrustState.
///
/// Per matrix-rust-sdk `OwnUserIdentityVerifiedState`: own identity
/// has three states. `withdraw_own_verification()` only transitions
/// `VerificationViolation → NeverVerified`, not `Verified → NeverVerified`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SelfVerificationState {
    /// Never verified (initial state, or after withdrawal).
    NeverVerified,
    /// Verified — our local seed matches the published root.
    Verified,
    /// Another device rotated our identity on the DHT — our local
    /// seed no longer derives the published root.
    VerificationViolation,
}

/// Result of identity origination. Holds everything needed for vault
/// persistence AND runtime construction.
///
/// **Typestate pattern:** vault data is accessible while the seed is
/// still owned. `into_identity()` consumes the struct, transferring
/// seed ownership to `SelfIdentity`. After consumption, the vault
/// data is no longer accessible — the caller must have stored it
/// before calling `into_identity()`.
///
/// The consumer flow:
/// ```ignore
/// let result = SelfIdentity::originate_new("VLD0:profile", Some("alice"))?;
/// vault.store_key("identity.signing-key", result.vault_seed())?;
/// vault.store_key("identity.x25519-seed", result.vault_x25519_seed())?;
/// let self_id = result.into_identity()?;
/// ```
pub struct OriginationResult {
    originated: crate::origin::originate::OriginatedIdentity,
    /// Pre-cloned revocation certificate (OriginatedIdentity is consumed by into_identity).
    pub revocation: crate::origin::originate::RevocationCertificate,
    /// Ed25519 public root — for DHT publication and FriendRequestEntry.
    pub root: IdentityRoot,
    /// X25519 DH public key bytes — for prekey bundle and profile publication.
    pub dh_public: [u8; 32],
    /// G2-derived X25519 seed — for vault persistence. Deterministically
    /// derivable from the signing seed, but stored separately so vault
    /// unlock doesn't need to re-derive.
    x25519_seed: [u8; 32],
    network: NetworkAddr,
    social: SocialProfile,
}

impl OriginationResult {
    /// Raw signing seed bytes for vault storage.
    /// Store this BEFORE calling `into_identity()`.
    pub fn vault_seed(&self) -> &[u8; 32] {
        self.originated.seed.vault_bytes()
    }

    /// G2-derived X25519 seed bytes for vault storage.
    pub fn vault_x25519_seed(&self) -> &[u8; 32] {
        &self.x25519_seed
    }

    /// Ed25519 public key as hex string.
    pub fn public_key_hex(&self) -> String {
        self.root.to_hex()
    }

    /// X25519 DH public key as hex — for profile publication.
    pub fn x25519_public_hex(&self) -> String {
        hex::encode(self.dh_public)
    }

    /// Update the profile key before consuming into SelfIdentity.
    /// Used when the profile DHT key is not known at origination time
    /// (created during init after the seed is generated).
    pub fn set_profile_key(&mut self, profile_key: &str) -> Result<(), IdentityError> {
        self.network = NetworkAddr::new(profile_key)?;
        Ok(())
    }

    /// Consume this result and produce the runtime `SelfIdentity`.
    /// Call this AFTER storing vault_seed() and vault_x25519_seed().
    pub fn into_identity(self) -> Result<SelfIdentity, IdentityError> {
        let dh = crate::operational::dh::dh_public_from_seed(&self.originated.dh_seed)?;
        let crypto = CryptoIdentity {
            root: self.originated.root,
            dh,
        };
        Ok(SelfIdentity {
            peer: PeerId::bind(crypto, self.network, self.social),
            seed: self.originated.seed,
            verification: SelfVerificationState::NeverVerified,
        })
    }
}

/// Our own identity — the private half that never crosses a wire.
///
/// `SelfIdentity` is a facade over the frozen derivation primitives
/// (G1–G6). Every method delegates; none inlines a domain tag. If you
/// see a `blake3::derive_key` call in this file with a string literal
/// that is NOT `derivation_tags::*`, the grammar has been forked.
pub struct SelfIdentity {
    peer: PeerId,
    seed: OriginSeed,
    verification: SelfVerificationState,
}

impl SelfIdentity {
    /// Construct from a freshly originated identity.
    pub fn from_originated(
        originated: OriginatedIdentity,
        network: NetworkAddr,
        social: SocialProfile,
    ) -> Result<Self, IdentityError> {
        let dh = crate::operational::dh::dh_public_from_seed(&originated.dh_seed)?;
        let crypto = CryptoIdentity {
            root: originated.root,
            dh,
        };
        Ok(Self {
            peer: PeerId::bind(crypto, network, social),
            seed: originated.seed,
            verification: SelfVerificationState::NeverVerified,
        })
    }

    /// Construct from a vault-restored identity.
    pub fn from_restored(
        restored: RestoredIdentity,
        network: NetworkAddr,
        social: SocialProfile,
    ) -> Result<Self, IdentityError> {
        let dh = crate::operational::dh::dh_public_from_seed(&restored.dh_seed)?;
        let crypto = CryptoIdentity {
            root: restored.root,
            dh,
        };
        Ok(Self {
            peer: PeerId::bind(crypto, network, social),
            seed: restored.seed,
            verification: SelfVerificationState::NeverVerified,
        })
    }

    /// Originate a new identity. Returns everything needed for both
    /// vault persistence and runtime operation in a single struct.
    ///
    /// The caller stores `result.vault_seed()` and `result.vault_x25519_seed()`
    /// to the vault, then calls `result.into_identity()` to get the
    /// `SelfIdentity`. The struct enforces the correct ordering: vault
    /// data is accessible while the seed is still owned, and
    /// `into_identity()` consumes the struct so the seed can't be
    /// accessed after the SelfIdentity takes ownership.
    pub fn originate_new(
        profile_key: &str,
        display_name: Option<&str>,
    ) -> Result<OriginationResult, IdentityError> {
        let originated = crate::origin::originate::originate()?;
        let revocation = originated.revocation.clone();
        let root = originated.root;
        let dh_public = originated.dh_public;
        let x25519_seed = blake3::derive_key(derivation_tags::DH_FROM_SEED, originated.seed.expose());
        let network = NetworkAddr::new(profile_key)?;
        let social = SocialProfile::with_cached_name(display_name.map(|s| s.to_owned()));

        Ok(OriginationResult {
            originated,
            revocation,
            root,
            dh_public,
            x25519_seed,
            network,
            social,
        })
    }

    /// Convenience: restore from vault-loaded seed bytes + profile key.
    pub fn restore_from_seed(
        seed_bytes: zeroize::Zeroizing<[u8; 32]>,
        profile_key: &str,
    ) -> Result<Self, IdentityError> {
        let seed = OriginSeed::from_vault_bytes(seed_bytes);
        let restored = crate::origin::originate::restore(seed)?;
        let network = NetworkAddr::new(profile_key)?;
        let social = SocialProfile::empty();
        let dh = crate::operational::dh::dh_public_from_seed(&restored.dh_seed)?;
        let crypto = CryptoIdentity { root: restored.root, dh };
        Ok(Self {
            peer: PeerId::bind(crypto, network, social),
            seed: restored.seed,
            verification: SelfVerificationState::NeverVerified,
        })
    }

    // ── Public half ────────────────────────────────────────────

    /// The public PeerId — safe to transmit, store, display.
    pub fn peer(&self) -> &PeerId {
        &self.peer
    }

    /// Convenience: the Ed25519 identity root.
    pub fn root(&self) -> &IdentityRoot {
        self.peer.root()
    }

    /// Convenience: the current rotation epoch.
    /// At origination this is ORIGIN (0).
    pub fn epoch(&self) -> RotationEpoch {
        RotationEpoch::ORIGIN // TODO: track rotation epoch in SelfIdentity
    }

    // ── Own verification state ─────────────────────────────────

    /// Current own verification state.
    pub fn verification_state(&self) -> SelfVerificationState {
        self.verification
    }

    /// Check if the published root on the DHT matches our local derivation.
    /// If it differs, another device rotated our identity — enter VerificationViolation.
    /// If it matches and we're not already in violation, mark as Verified.
    pub fn check_public_identity(&mut self, observed_root: &IdentityRoot) {
        if observed_root == self.root() {
            if self.verification != SelfVerificationState::VerificationViolation {
                self.verification = SelfVerificationState::Verified;
            }
        } else {
            tracing::debug!(
                local = ?&self.root().as_bytes()[..4],
                observed = ?&observed_root.as_bytes()[..4],
                "own identity mismatch — another device rotated our identity"
            );
            self.verification = SelfVerificationState::VerificationViolation;
        }
    }

    /// Withdraw own verification — only transitions VerificationViolation → NeverVerified.
    /// Per matrix-rust-sdk: you cannot withdraw from Verified (nothing to withdraw).
    pub fn withdraw_own_verification(&mut self) {
        if self.verification == SelfVerificationState::VerificationViolation {
            self.verification = SelfVerificationState::NeverVerified;
        }
    }

    /// Mark own identity as verified (e.g., after importing seed on a new device
    /// and confirming it matches the published root).
    pub fn mark_own_verified(&mut self) {
        self.verification = SelfVerificationState::Verified;
    }

    // ── Derivation facades ─────────────────────────────────────
    //
    // Each method replaces a scattered inline derivation in rekindle-chat.
    // The body delegates to an existing frozen function. If you typed a
    // domain tag string literal in any of these methods, you have forked
    // the derivation grammar.

    /// G3: pseudonym seed for a community.
    ///
    /// **Kills bug 3 by construction:** takes `&GovernanceKey` which
    /// strips the VLD0: prefix via `canonical_bytes()` and includes the
    /// substrate discriminant byte in IKM per A-7. The old
    /// `SigningKeyHandle::pseudonym_seed(&str)` that interpolated the
    /// governance key into the context string is deleted, not patched.
    pub fn pseudonym_seed(&self, gov: &GovernanceKey) -> Zeroizing<[u8; 32]> {
        crate::projection::pseudonym::pseudonym_seed_from_origin(&self.seed, gov)
    }

    /// G3+G4+G5: derive a full community persona.
    ///
    /// Delegates to `derive_persona` — the ONLY pseudonym producer.
    pub fn derive_persona(
        &self,
        gov: &GovernanceKey,
        slot: u32,
    ) -> Result<(CommunityPersona, PersonaSecrets), IdentityError> {
        crate::projection::pseudonym::derive_persona(&self.seed, gov, slot)
    }

    /// G2: X25519 DH seed.
    ///
    /// Delegates to the frozen DH_FROM_SEED derivation tag.
    pub fn dh_seed(&self) -> DhSeed {
        let raw = blake3::derive_key(derivation_tags::DH_FROM_SEED, self.seed.expose());
        DhSeed::from_raw(Zeroizing::new(raw))
    }

    /// G6: canonical session anchor with a peer.
    ///
    /// **Kills bug 2 by construction:** the ONLY chat-layer path to a
    /// session ID. Both sides call this with typed `&IdentityRoot` values
    /// (not hex strings, not seeds), and `session_anchor` applies
    /// lexicographic ordering internally. The old inline
    /// `BLAKE3(our_signing_seed || peer_pubkey_bytes)` without ordering
    /// is deleted, not patched.
    pub fn session_anchor_with(
        &self,
        peer_root: &IdentityRoot,
    ) -> Result<SessionAnchor, IdentityError> {
        session_anchor(self.peer.root(), peer_root)
    }

    /// Reconstruct the Ed25519 signing keypair for PQXDH operations.
    ///
    /// Returns an opaque `SigningKeypair` with methods for all signing
    /// operations: `sign_raw()`, `sign_ec_prekey()`, `sign_pq_prekey()`,
    /// `public_key_bytes()`. The inner aws-lc-rs type is not exposed.
    ///
    /// The keypair is reconstructed from the seed on every call and
    /// should be dropped promptly after use. For batch operations
    /// (signing 100 one-time prekeys), construct once and call
    /// `sign_pq_prekey()` in a loop.
    pub fn signing_keypair(&self) -> Result<crate::signing::SigningKeypair, IdentityError> {
        let inner = sign::keypair_from_seed(self.seed.expose())
            .map_err(|e| IdentityError::KeypairDerivation {
                reason: format!("signing_keypair: {e}"),
            })?;
        Ok(crate::signing::SigningKeypair::from_inner(inner))
    }

    /// Return the X25519 identity seed bytes for PQXDH initiate/respond.
    ///
    /// The PQXDH flow needs raw `&[u8; 32]` as `ik_dh_seed` to construct
    /// `agreement::PrivateKey` via `reusable_from_seed()`. This returns
    /// the G2-derived X25519 seed wrapped in `Zeroizing` for auto-cleanup.
    pub fn x25519_identity_seed(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(blake3::derive_key(derivation_tags::DH_FROM_SEED, self.seed.expose()))
    }

    /// Sign arbitrary bytes with the identity root key.
    ///
    /// Delegates to `rekindle_ratchet::crypto::sign::sign_raw`.
    /// The keypair is reconstructed from the seed, used once, and dropped.
    pub fn sign(&self, msg: &[u8]) -> Result<Signature64, IdentityError> {
        let kp = sign::keypair_from_seed(self.seed.expose())
            .map_err(|e| IdentityError::KeypairDerivation {
                reason: format!("sign: {e}"),
            })?;
        let sig = sign::sign_raw(&kp, msg);
        Ok(Signature64::from_bytes(sig))
    }
}

// SelfIdentity is intentionally:
// - NOT Clone (OriginSeed is !Clone)
// - NOT Serialize/Deserialize (seed must never be on a wire without vault encryption)
// - NOT PartialEq/Eq (comparison of secret material is meaningless)

impl core::fmt::Debug for SelfIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SelfIdentity")
            .field("peer", &self.peer)
            .field("verification", &self.verification)
            .field("seed", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locator::ProfileLocator;
    use crate::origin::originate::originate_from_seed;
    use zeroize::Zeroizing;

    fn make_self_identity(byte: u8) -> SelfIdentity {
        let seed = OriginSeed::from_vault_bytes(Zeroizing::new([byte; 32]));
        let originated = originate_from_seed(seed).unwrap();
        let network = NetworkAddr {
            profile: ProfileLocator::parse("VLD0:testprofile").unwrap(),
            mailbox: None,
            inbox: None,
            locator_epoch: 0,
        };
        let social = SocialProfile { display: None, cached_name: None };
        SelfIdentity::from_originated(originated, network, social).unwrap()
    }

    #[test]
    fn originate_new_typestate() {
        let result = SelfIdentity::originate_new(
            "VLD0:testprofile", Some("alice"),
        ).unwrap();

        // Vault data accessible before consuming
        assert_ne!(result.vault_seed(), &[0u8; 32]);
        assert_ne!(result.vault_x25519_seed(), &[0u8; 32]);
        assert_eq!(result.revocation.root, result.root);
        assert_eq!(result.public_key_hex().len(), 64);

        // Consume into SelfIdentity
        let si = result.into_identity().unwrap();
        assert_eq!(si.verification_state(), SelfVerificationState::NeverVerified);
        // result.vault_seed() is no longer accessible — struct consumed
    }

    #[test]
    fn restore_from_seed_convenience() {
        let si = SelfIdentity::restore_from_seed(
            Zeroizing::new([0x42; 32]),
            "VLD0:testprofile",
        ).unwrap();
        assert_ne!(si.root().as_bytes(), &[0u8; 32]);
        assert_eq!(si.verification_state(), SelfVerificationState::NeverVerified);
    }

    #[test]
    fn peer_id_accessible() {
        let si = make_self_identity(0x01);
        assert_ne!(si.root().as_bytes(), &[0u8; 32]);
        assert_eq!(si.peer().root(), si.root());
    }

    #[test]
    fn own_verification_initial_state() {
        let si = make_self_identity(0x01);
        assert_eq!(si.verification_state(), SelfVerificationState::NeverVerified);
    }

    #[test]
    fn own_verification_matches_published_root() {
        let mut si = make_self_identity(0x01);
        let root = *si.root();
        si.check_public_identity(&root);
        assert_eq!(si.verification_state(), SelfVerificationState::Verified);
    }

    #[test]
    fn own_verification_mismatch_enters_violation() {
        let mut si = make_self_identity(0x01);
        let other_si = make_self_identity(0x02);
        let other_root = *other_si.root();
        si.check_public_identity(&other_root);
        assert_eq!(si.verification_state(), SelfVerificationState::VerificationViolation);
    }

    #[test]
    fn own_withdraw_only_from_violation() {
        let mut si = make_self_identity(0x01);
        // Withdraw from NeverVerified — no-op
        si.withdraw_own_verification();
        assert_eq!(si.verification_state(), SelfVerificationState::NeverVerified);

        // Enter Verified, withdraw — no-op
        si.mark_own_verified();
        si.withdraw_own_verification();
        assert_eq!(si.verification_state(), SelfVerificationState::Verified);

        // Enter Violation, withdraw — transitions to NeverVerified
        let other_si = make_self_identity(0x02);
        let other_root = *other_si.root();
        si.check_public_identity(&other_root);
        assert_eq!(si.verification_state(), SelfVerificationState::VerificationViolation);
        si.withdraw_own_verification();
        assert_eq!(si.verification_state(), SelfVerificationState::NeverVerified);
    }

    #[test]
    fn pseudonym_seed_delegates_to_g3() {
        let si = make_self_identity(0x01);
        let gov = GovernanceKey::parse("VLD0:testcommunity").unwrap();
        let seed_a = si.pseudonym_seed(&gov);

        let si2 = make_self_identity(0x01);
        let seed_b = si2.pseudonym_seed(&gov);
        assert_eq!(*seed_a, *seed_b, "pseudonym seed must be deterministic");
    }

    #[test]
    fn pseudonym_seed_canonicalizes_governance_key() {
        let si = make_self_identity(0x01);
        let gov_prefixed = GovernanceKey::parse("VLD0:mykey").unwrap();

        let si2 = make_self_identity(0x01);
        let gov_bare = GovernanceKey::parse("mykey").unwrap();

        assert_eq!(
            *si.pseudonym_seed(&gov_prefixed),
            *si2.pseudonym_seed(&gov_bare),
            "same canonical bytes must produce same seed regardless of prefix"
        );
    }

    #[test]
    fn session_anchor_symmetric() {
        let si_a = make_self_identity(0x01);
        let si_b = make_self_identity(0x02);

        let anchor_ab = si_a.session_anchor_with(si_b.root()).unwrap();
        let anchor_ba = si_b.session_anchor_with(si_a.root()).unwrap();
        assert_eq!(anchor_ab, anchor_ba, "session anchor must be symmetric");
    }

    #[test]
    fn session_anchor_self_rejected() {
        let si = make_self_identity(0x01);
        let err = si.session_anchor_with(si.root());
        assert!(err.is_err(), "self-session must be rejected");
    }

    #[test]
    fn signing_keypair_produces_matching_root() {
        let si = make_self_identity(0x01);
        let kp = si.signing_keypair().unwrap();
        assert_eq!(&kp.public_key_bytes(), si.root().as_bytes(),
            "keypair public key must match the identity root");
    }

    #[test]
    fn x25519_identity_seed_matches_g2() {
        let si = make_self_identity(0x42);
        let x_seed = si.x25519_identity_seed();
        let expected = blake3::derive_key(derivation_tags::DH_FROM_SEED, &[0x42u8; 32]);
        assert_eq!(*x_seed, expected,
            "x25519_identity_seed must match the G2 derivation");
    }

    #[test]
    fn dh_seed_matches_g2_convention() {
        let si = make_self_identity(0x42);
        let dh_seed = si.dh_seed();
        let expected = blake3::derive_key(derivation_tags::DH_FROM_SEED, &[0x42u8; 32]);
        assert_eq!(dh_seed.expose(), &expected);
    }

    #[test]
    fn sign_and_verify() {
        let si = make_self_identity(0x01);
        let msg = b"hello rekindle";
        let sig = si.sign(msg).unwrap();

        let result = sign::verify_raw(
            si.root().as_bytes(),
            msg,
            sig.as_bytes(),
        );
        assert!(result.is_ok(), "signature must verify against the root");
    }

    #[test]
    fn derive_persona_delegates() {
        let si = make_self_identity(0x01);
        let gov = GovernanceKey::parse("VLD0:testcommunity").unwrap();
        let (persona, _secrets) = si.derive_persona(&gov, 5).unwrap();
        assert_eq!(persona.slot_index, 5);
        assert_eq!(persona.governance, gov);
    }

    #[test]
    fn debug_redacts_seed() {
        let si = make_self_identity(0x01);
        let dbg = format!("{si:?}");
        assert!(dbg.contains("<redacted>"), "Debug must redact the seed");
        assert!(!dbg.contains("OriginSeed"), "Debug must not leak seed type name");
    }
}
