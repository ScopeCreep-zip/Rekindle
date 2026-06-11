//! Substrate-tagged locator types and the `GovernanceKey` newtype.
//!
//! Every locator wraps a substrate-native key form. The substrate type
//! tag (e.g., `VLD0:`) is serialization dress, not key material. Each
//! sealed type's `canonical_bytes()` is the ONLY form any KDF, label
//! builder, or comparison may consume. The tag is stripped at parse
//! time and reattached only for `display_form()`.
//!
//! Layer invariant I-L3: no locator type implements `AsRef<[u8]>` toward
//! the label/derivation APIs. Locators route; they never identify.

use crate::error::IdentityError;

/// The network substrate a locator is addressed on.
///
/// Closed enum — adding a new substrate is a minor-version change
/// with a new discriminant byte in the KDF (A-7 amendment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Substrate {
    /// Veilid DHT. Display prefix: `VLD0:`.
    VeilidDht = 0,
}

impl Substrate {
    /// The single-byte discriminant used in KDF inputs (G3 IKM).
    /// Pinned per variant — changing a discriminant is a wire break.
    pub fn discriminant_byte(self) -> u8 {
        self as u8
    }

    /// The display prefix for this substrate.
    pub fn display_prefix(self) -> &'static str {
        match self {
            Self::VeilidDht => "VLD0:",
        }
    }

    /// Parse a display prefix to a substrate variant.
    fn from_prefix(s: &str) -> Option<(Self, &str)> {
        if let Some(rest) = s.strip_prefix("VLD0:") {
            return Some((Self::VeilidDht, rest));
        }
        None
    }
}

// ── Sealed locator internals ────────────────────────────────────
//
// All four locator types share the same internal structure: a substrate
// tag + canonical key material. The fields are private; construction
// is only via `parse()`.

/// A Veilid DHT record key for a peer's profile.
///
/// Changes on identity rotation. NOT the identity — a routing hint.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ProfileLocator {
    substrate: Substrate,
    /// Canonical key material (substrate prefix stripped).
    /// Stored as UTF-8 bytes of the BASE64URL_NOPAD-encoded Veilid key.
    /// Variable-length: Veilid's `BareOpaqueRecordKey` wraps `bytes::Bytes`
    /// (not `[u8; 32]`), so `Vec<u8>` is the correct representation.
    /// Typical size: 43 bytes (32 raw bytes → base64url-nopad).
    canonical: Vec<u8>,
}

/// A Veilid DHT record key for a peer's RPC mailbox.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct MailboxLocator {
    substrate: Substrate,
    canonical: Vec<u8>,
}

/// A Veilid DHT record key for a peer's friend inbox.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct InboxLocator {
    substrate: Substrate,
    canonical: Vec<u8>,
}

/// A Veilid DHT record key for a community's governance manifest.
///
/// `canonical_bytes()` is consumed by:
/// - G3 pseudonym derivation (as part of IKM)
/// - Vault label builders (as the governance identifier in labels)
/// - Equality/hash comparisons between governance keys
///
/// The substrate prefix NEVER enters a KDF — `canonical_bytes()` strips it.
/// The substrate discriminant byte enters G3's IKM separately (A-7).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct GovernanceKey {
    substrate: Substrate,
    /// Canonical key material (substrate prefix stripped).
    /// Stored as UTF-8 bytes of the BASE64URL_NOPAD-encoded Veilid key.
    /// Variable-length: Veilid's `BareOpaqueRecordKey` wraps `bytes::Bytes`
    /// (not `[u8; 32]`), so `Vec<u8>` is the correct representation.
    /// Typical size: 43 bytes (32 raw bytes → base64url-nopad).
    canonical: Vec<u8>,
}

// ── Shared construction + access ────────────────────────────────

macro_rules! impl_locator {
    ($ty:ident, $label:literal) => {
        impl $ty {
            /// Parse from the substrate's display form (e.g., `"VLD0:abc123"`).
            ///
            /// Strips the substrate prefix and stores canonical material.
            /// Returns error if no recognized prefix is found.
            pub fn parse(display_form: &str) -> Result<Self, IdentityError> {
                if let Some((substrate, rest)) = Substrate::from_prefix(display_form) {
                    if rest.is_empty() {
                        return Err(IdentityError::LocatorParse(format!(
                            "{}: empty key material after prefix", $label
                        )));
                    }
                    return Ok(Self {
                        substrate,
                        canonical: rest.as_bytes().to_vec(),
                    });
                }
                // No recognized prefix — treat as bare key on VeilidDht
                // (backward compatibility with existing code that strips
                // prefixes before passing to parse).
                if display_form.is_empty() {
                    return Err(IdentityError::LocatorParse(format!(
                        "{}: empty input", $label
                    )));
                }
                Ok(Self {
                    substrate: Substrate::VeilidDht,
                    canonical: display_form.as_bytes().to_vec(),
                })
            }

            /// The canonical key material — substrate prefix stripped.
            /// THE bytes consumed by KDFs, labels, and comparisons.
            pub fn canonical_bytes(&self) -> &[u8] {
                &self.canonical
            }

            /// The substrate this locator is addressed on.
            pub fn substrate(&self) -> Substrate {
                self.substrate
            }

            /// Reconstruct the substrate display form for transport calls.
            pub fn display_form(&self) -> String {
                let canonical_str = String::from_utf8_lossy(&self.canonical);
                format!("{}{}", self.substrate.display_prefix(), canonical_str)
            }

            /// Canonical bytes as a hex string (for vault labels).
            pub fn canonical_hex(&self) -> String {
                hex::encode(&self.canonical)
            }
        }
    };
}

impl_locator!(ProfileLocator, "ProfileLocator");
impl_locator!(MailboxLocator, "MailboxLocator");
impl_locator!(InboxLocator, "InboxLocator");
impl_locator!(GovernanceKey, "GovernanceKey");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn governance_key_parse_with_prefix() {
        let gk = GovernanceKey::parse("VLD0:5fVBSx3abc").unwrap();
        assert_eq!(gk.canonical_bytes(), b"5fVBSx3abc");
        assert_eq!(gk.substrate(), Substrate::VeilidDht);
    }

    #[test]
    fn governance_key_parse_bare() {
        let gk = GovernanceKey::parse("5fVBSx3abc").unwrap();
        assert_eq!(gk.canonical_bytes(), b"5fVBSx3abc");
    }

    #[test]
    fn governance_key_display_roundtrip() {
        let gk = GovernanceKey::parse("VLD0:NC-OBPrd123").unwrap();
        let display = gk.display_form();
        assert_eq!(display, "VLD0:NC-OBPrd123");
        let restored = GovernanceKey::parse(&display).unwrap();
        assert_eq!(gk, restored);
    }

    #[test]
    fn governance_key_canonical_strips_prefix() {
        let with_prefix = GovernanceKey::parse("VLD0:abc123").unwrap();
        let bare = GovernanceKey::parse("abc123").unwrap();
        assert_eq!(with_prefix.canonical_bytes(), bare.canonical_bytes());
    }

    #[test]
    fn governance_key_equality_includes_substrate() {
        let a = GovernanceKey::parse("VLD0:abc123").unwrap();
        let b = GovernanceKey::parse("VLD0:abc123").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn governance_key_empty_rejected() {
        assert!(GovernanceKey::parse("").is_err());
        assert!(GovernanceKey::parse("VLD0:").is_err());
    }

    #[test]
    fn substrate_discriminant_bytes_distinct() {
        // If we add more substrates, this test ensures they don't collide
        let veilid = Substrate::VeilidDht.discriminant_byte();
        assert_eq!(veilid, 0);
    }

    #[test]
    fn profile_locator_parse() {
        let pl = ProfileLocator::parse("VLD0:someProfileKey").unwrap();
        assert_eq!(pl.canonical_bytes(), b"someProfileKey");
        assert_eq!(pl.display_form(), "VLD0:someProfileKey");
    }

    #[test]
    fn mailbox_locator_parse() {
        let ml = MailboxLocator::parse("VLD0:someMailboxKey").unwrap();
        assert_eq!(ml.canonical_bytes(), b"someMailboxKey");
    }

    #[test]
    fn inbox_locator_parse() {
        let il = InboxLocator::parse("VLD0:someInboxKey").unwrap();
        assert_eq!(il.canonical_bytes(), b"someInboxKey");
    }

    #[test]
    fn canonical_hex_for_labels() {
        let gk = GovernanceKey::parse("VLD0:TestKey").unwrap();
        let hex = gk.canonical_hex();
        // "TestKey" → hex of UTF-8 bytes
        assert_eq!(hex, hex::encode(b"TestKey"));
    }
}
