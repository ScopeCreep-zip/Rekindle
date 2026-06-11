//! PeerId — the composite identity type that binds all layers.
//!
//! `PeerId` is the single type that flows through friendship, community,
//! and messaging pipelines. It binds cryptographic root (Layer 0),
//! network addressing (Layer 1), and social presentation (Layer 2)
//! into one value with root-only equality semantics.
//!
//! Construction: `PeerId::bind()` only — the single `PeerRef::anchor()`
//! call site outside the trust store.
//!
//! Equality: `PartialEq`/`Eq`/`Hash` derive from `anchor` (root bytes)
//! ONLY. Network and social fields are mutable and do NOT participate.

use crate::error::IdentityError;
use crate::locator::{ProfileLocator, MailboxLocator, InboxLocator};
use crate::operational::DhKey;
use crate::origin::originate::{IdentityRoot, PeerRef};
use crate::persona::DisplayName;

// ── CryptoIdentity (Layer 0) ───────────────────────────────────

/// The cryptographic root pair. Equality basis for PeerId.
///
/// `root` is Ed25519 — THE identity. `dh` is X25519 — derived via G2,
/// never the Eq basis. The type distinction (IdentityRoot vs DhKey)
/// prevents passing one where the other is expected at compile time.
#[derive(Debug, Clone)]
pub struct CryptoIdentity {
    pub root: IdentityRoot,
    pub dh: DhKey,
}

// ── NetworkAddr (Layer 1) ──────────────────────────────────────

/// Network addressing for a peer. Mutable — changes on rotation,
/// node migration, or route refresh. NOT part of Eq/Hash.
///
/// `locator_epoch` is monotonic per root chain. A cached NetworkAddr
/// with a lower epoch is stale and must not overwrite a higher one.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NetworkAddr {
    pub profile: ProfileLocator,
    pub mailbox: Option<MailboxLocator>,
    pub inbox: Option<InboxLocator>,
    /// Monotonic staleness ordering. Two cached copies are orderable.
    pub locator_epoch: u64,
}

impl NetworkAddr {
    /// Convenience: parse a profile key string, default everything else.
    ///
    /// The 90% case — most callers have only a profile key at construction
    /// time. Mailbox/inbox are discovered later. Adding optional fields
    /// to NetworkAddr later changes this function, not every caller.
    pub fn new(profile_key: &str) -> Result<Self, IdentityError> {
        Ok(Self {
            profile: ProfileLocator::parse(profile_key)?,
            mailbox: None,
            inbox: None,
            locator_epoch: 0,
        })
    }
}

// ── SocialProfile (Layer 2) ────────────────────────────────────

/// Social presentation metadata. Mutable. NOT part of Eq/Hash.
/// Advisory only — never authoritative for routing or security.
#[derive(Debug, Clone)]
pub struct SocialProfile {
    /// Signed display name claim. Optional until first publication.
    pub display: Option<DisplayName>,
    /// Cached display name string for rendering when no signed claim
    /// is available (e.g., deserialized from session_meta before the
    /// peer publishes a signed DisplayName). Advisory, never trusted.
    pub cached_name: Option<String>,
}

impl SocialProfile {
    /// Empty social profile — no display name, no cache.
    pub fn empty() -> Self {
        Self { display: None, cached_name: None }
    }

    /// Social profile with a cached (unsigned) display name.
    pub fn with_cached_name(name: Option<String>) -> Self {
        Self { display: None, cached_name: name }
    }

    /// Social profile with a signed display name claim.
    pub fn with_display(display: DisplayName) -> Self {
        Self { display: Some(display), cached_name: None }
    }
}

// ── PeerId ─────────────────────────────────────────────────────

/// The complete peer record: crypto + network + social.
///
/// **Equality is root-only.** Two `PeerId`s with the same root but
/// different network addresses (stale vs fresh locator) are equal.
/// This prevents map corruption when a peer's locator changes.
///
/// **Construction is sealed.** The `anchor` field is private. The only
/// way to construct a `PeerId` is `PeerId::bind()`, which mints the
/// `PeerRef` once. This is a sanctioned `PeerRef::anchor()` call site
/// (WS-0 source-scan allowlist entry).
///
/// **Serde:** `Serialize`/`Deserialize` emit crypto.root as hex,
/// deserialize through `IdentityRoot::from_bytes()` validation (rejects
/// all-zeros/off-curve), and reconstruct via `bind()`. The `anchor`
/// field is never serialized directly.
pub struct PeerId {
    /// PRIVATE. The equality/hash basis. Set once at bind().
    anchor: PeerRef,
    /// Layer 0: cryptographic root pair.
    pub crypto: CryptoIdentity,
    /// Layer 1: network addressing. Mutable, not Eq.
    pub network: NetworkAddr,
    /// Layer 2: social presentation. Mutable, not Eq.
    pub social: SocialProfile,
    /// Forward-compatibility version. Old nodes that don't understand
    /// new fields can still resolve, store, and forward the PeerId.
    pub version: u8,
}

impl PeerId {
    /// The ONLY constructor. Mints a `PeerRef` from `crypto.root`.
    ///
    /// This is a sanctioned `PeerRef::anchor()` call site.
    /// The WS-0 source-scan test allowlists this function.
    pub fn bind(
        crypto: CryptoIdentity,
        network: NetworkAddr,
        social: SocialProfile,
    ) -> Self {
        Self {
            anchor: PeerRef::anchor(crypto.root), // ← allowlisted anchor mint
            crypto,
            network,
            social,
            version: 1,
        }
    }

    /// The stable anchor reference. Used for trust store lookups,
    /// map keying, and anywhere a `&PeerRef` is required.
    pub fn peer_ref(&self) -> &PeerRef {
        &self.anchor
    }

    /// Convenience: the Ed25519 identity root.
    pub fn root(&self) -> &IdentityRoot {
        &self.crypto.root
    }

    /// Update network addressing. Rejects stale epochs.
    ///
    /// A NetworkAddr with `locator_epoch` lower than the current value
    /// is a replay or stale cache — reject with `StaleLocator` error.
    /// Does NOT touch crypto or anchor.
    pub fn update_network(&mut self, new: NetworkAddr) -> Result<(), IdentityError> {
        if new.locator_epoch < self.network.locator_epoch {
            return Err(IdentityError::StaleLocator {
                presented: new.locator_epoch,
                known: self.network.locator_epoch,
            });
        }
        self.network = new;
        Ok(())
    }

    /// Update social profile. No validation — display names are advisory.
    pub fn update_social(&mut self, social: SocialProfile) {
        self.social = social;
    }

    /// Convenience: construct from an `OriginatedIdentity` + profile key string.
    ///
    /// Derives the DH public key, parses the profile locator, and binds.
    /// This is the 90% path for identity origination — one call instead of
    /// 8 imports and 15 lines of struct assembly. Adding fields to
    /// `NetworkAddr` or `SocialProfile` changes this function, not 55 call sites.
    pub fn from_originated(
        originated: &crate::origin::originate::OriginatedIdentity,
        profile_key: &str,
    ) -> Result<Self, IdentityError> {
        let dh = crate::operational::dh::dh_public_from_seed(&originated.dh_seed)?;
        let crypto = CryptoIdentity { root: originated.root, dh };
        let network = NetworkAddr::new(profile_key)?;
        let social = SocialProfile::empty();
        Ok(Self::bind(crypto, network, social))
    }

    /// Convenience: construct from raw root + DH key + profile key.
    ///
    /// For cases where the caller has the parts but not an OriginatedIdentity
    /// (e.g., deserializing from a friend request entry or community member list).
    pub fn from_parts(
        root: IdentityRoot,
        dh: DhKey,
        profile_key: &str,
        display_name: Option<String>,
    ) -> Result<Self, IdentityError> {
        let crypto = CryptoIdentity { root, dh };
        let network = NetworkAddr::new(profile_key)?;
        let social = SocialProfile::with_cached_name(display_name);
        Ok(Self::bind(crypto, network, social))
    }
}

// ── Equality: root-only, hand-impl ─────────────────────────────

impl PartialEq for PeerId {
    fn eq(&self, other: &Self) -> bool {
        self.anchor == other.anchor
    }
}

impl Eq for PeerId {}

impl std::hash::Hash for PeerId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.anchor.hash(state);
    }
}

impl Clone for PeerId {
    fn clone(&self) -> Self {
        Self {
            anchor: self.anchor,
            crypto: self.crypto.clone(),
            network: self.network.clone(),
            social: self.social.clone(),
            version: self.version,
        }
    }
}

impl core::fmt::Debug for PeerId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PeerId")
            .field("root", &self.crypto.root)
            .field("network.profile", &self.network.profile.display_form())
            .field("version", &self.version)
            .finish()
    }
}

// ── Serde: serialize root as hex, deserialize through validation ─

impl serde::Serialize for PeerId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("PeerId", 5)?;
        s.serialize_field("root", &self.crypto.root)?;
        s.serialize_field("dh", &self.crypto.dh)?;
        s.serialize_field("network", &self.network)?;
        // Prefer signed display name; fall back to cached name
        let name = self.social.display.as_ref()
            .map(|d| d.name.as_str())
            .or(self.social.cached_name.as_deref());
        s.serialize_field("display_name", &name)?;
        s.serialize_field("version", &self.version)?;
        s.end()
    }
}

impl<'de> serde::Deserialize<'de> for PeerId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct PeerIdRaw {
            root: IdentityRoot,
            dh: DhKey,
            network: NetworkAddr,
            #[serde(default)]
            display_name: Option<String>,
            #[serde(default = "default_version")]
            version: u8,
        }

        fn default_version() -> u8 { 1 }

        let raw = PeerIdRaw::deserialize(deserializer)?;

        // Reconstruction via bind() — never deserializes PeerRef directly.
        // IdentityRoot::deserialize already rejects all-zeros via from_hex → from_bytes.
        let crypto = CryptoIdentity { root: raw.root, dh: raw.dh };
        let social = SocialProfile {
            display: None,
            cached_name: raw.display_name,
        };
        let mut peer = PeerId::bind(crypto, raw.network, social);
        peer.version = raw.version;
        Ok(peer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::originate_from_seed;
    use crate::origin::seed::OriginSeed;
    use crate::operational::dh::dh_public_from_seed;
    use std::collections::HashSet;
    use zeroize::Zeroizing;

    fn make_peer(byte: u8) -> PeerId {
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([byte; 32]))
        ).unwrap();
        let dh = dh_public_from_seed(&o.dh_seed).unwrap();
        let crypto = CryptoIdentity { root: o.root, dh };
        let network = NetworkAddr {
            profile: ProfileLocator::parse("VLD0:testprofile").unwrap(),
            mailbox: None,
            inbox: None,
            locator_epoch: 0,
        };
        let social = SocialProfile { display: None, cached_name: None };
        PeerId::bind(crypto, network, social)
    }

    #[test]
    fn equality_is_root_only() {
        let a = make_peer(0x01);
        let mut b = a.clone();
        // Change network — equality must still hold
        b.network = NetworkAddr {
            profile: ProfileLocator::parse("VLD0:different").unwrap(),
            mailbox: Some(MailboxLocator::parse("VLD0:mailbox").unwrap()),
            inbox: None,
            locator_epoch: 99,
        };
        assert_eq!(a, b, "PeerId equality must be root-only");

        let mut set = HashSet::new();
        set.insert(a.clone());
        set.insert(b);
        assert_eq!(set.len(), 1, "same root must hash to one entry");
    }

    #[test]
    fn different_roots_are_different_peers() {
        let a = make_peer(0x01);
        let b = make_peer(0x02);
        assert_ne!(a, b);
    }

    #[test]
    fn update_network_rejects_stale_epoch() {
        let mut peer = make_peer(0x01);
        peer.network.locator_epoch = 5;

        let stale = NetworkAddr {
            profile: ProfileLocator::parse("VLD0:stale").unwrap(),
            mailbox: None,
            inbox: None,
            locator_epoch: 3, // lower than current 5
        };
        let err = peer.update_network(stale).unwrap_err();
        assert!(matches!(err, IdentityError::StaleLocator { presented: 3, known: 5 }));
    }

    #[test]
    fn update_network_accepts_equal_or_higher_epoch() {
        let mut peer = make_peer(0x01);
        peer.network.locator_epoch = 5;

        let same = NetworkAddr {
            profile: ProfileLocator::parse("VLD0:same").unwrap(),
            mailbox: None,
            inbox: None,
            locator_epoch: 5,
        };
        assert!(peer.update_network(same).is_ok());

        let higher = NetworkAddr {
            profile: ProfileLocator::parse("VLD0:higher").unwrap(),
            mailbox: None,
            inbox: None,
            locator_epoch: 6,
        };
        assert!(peer.update_network(higher).is_ok());
        assert_eq!(peer.network.locator_epoch, 6);
    }

    #[test]
    fn peer_ref_accessible() {
        let peer = make_peer(0x01);
        assert_eq!(*peer.peer_ref().root(), *peer.root());
    }

    #[test]
    fn serde_roundtrip() {
        let peer = make_peer(0x01);
        let json = serde_json::to_string(&peer).unwrap();
        let restored: PeerId = serde_json::from_str(&json).unwrap();
        assert_eq!(peer, restored, "serde roundtrip must preserve equality");
        assert_eq!(peer.root(), restored.root());
    }

    #[test]
    fn deserialize_rejects_invalid_root() {
        // All-zeros root should fail IdentityRoot::from_hex → from_bytes
        let bad_json = r#"{"root":"0000000000000000000000000000000000000000000000000000000000000000","dh":"abababababababababababababababababababababababababababababababababab","network":{"profile":{"substrate":"VeilidDht","canonical":[116,101,115,116]},"mailbox":null,"inbox":null,"locator_epoch":0},"version":1}"#;
        let result: Result<PeerId, _> = serde_json::from_str(bad_json);
        assert!(result.is_err(), "all-zeros root must fail deserialization");
    }

    #[test]
    fn from_originated_convenience() {
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]))
        ).unwrap();
        let peer = PeerId::from_originated(&o, "VLD0:testprofile").unwrap();
        assert_eq!(*peer.root(), o.root);
        assert_eq!(peer.network.locator_epoch, 0);
    }

    #[test]
    fn from_parts_convenience() {
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]))
        ).unwrap();
        let dh = dh_public_from_seed(&o.dh_seed).unwrap();
        let peer = PeerId::from_parts(
            o.root, dh, "VLD0:testprofile", Some("alice".into()),
        ).unwrap();
        assert_eq!(*peer.root(), o.root);
        assert_eq!(peer.social.cached_name.as_deref(), Some("alice"));
    }

    #[test]
    fn network_addr_new_convenience() {
        let addr = NetworkAddr::new("VLD0:testkey").unwrap();
        assert_eq!(addr.locator_epoch, 0);
        assert!(addr.mailbox.is_none());
        assert!(addr.inbox.is_none());
    }

    #[test]
    fn social_profile_empty() {
        let s = SocialProfile::empty();
        assert!(s.display.is_none());
        assert!(s.cached_name.is_none());
    }

    #[test]
    fn social_profile_with_cached_name() {
        let s = SocialProfile::with_cached_name(Some("bob".into()));
        assert_eq!(s.cached_name.as_deref(), Some("bob"));
        assert!(s.display.is_none());
    }

    #[test]
    fn bind_is_deterministic() {
        let a = make_peer(0x01);
        let b = make_peer(0x01);
        assert_eq!(a, b);
        assert_eq!(a.peer_ref(), b.peer_ref());
    }
}
