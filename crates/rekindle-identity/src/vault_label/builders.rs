//! Exhaustive, closed set of `Label` constructors.
//!
//! Every builder accepts ONLY sealed key types — `&IdentityRoot`,
//! `&GovernanceKey`, `&SessionAnchor`. No `&str` input exists in this
//! module. Full-width hex (64 chars for 32-byte keys). Zero truncation.
//!
//! Adding a new label requires adding a builder here. No other module
//! may construct `Label` values.

use super::label::Label;
use crate::locator::GovernanceKey;
use crate::origin::originate::IdentityRoot;
use crate::session::SessionAnchor;

// ── Per-identity vault labels (D-16: vault instantiated per IdentityRoot) ──
//
// Labels within a vault need no identity prefix — the vault itself is
// per-identity. Grammar: {ns}.{sub}[.{key64hex}][.{index}]

/// The origin seed storage label.
pub fn origin_seed() -> Label {
    Label::new_validated("origin.seed".into())
}

/// The DH seed storage label.
pub fn dh_seed() -> Label {
    Label::new_validated("origin.dh-seed".into())
}

/// The pre-committed revocation certificate.
pub fn revocation_certificate() -> Label {
    Label::new_validated("origin.revocation-cert".into())
}

// ── Prekey labels (per-peer, keyed by peer's IdentityRoot) ──────────

/// Signed prekey for a specific peer.
/// Full 64 hex chars of the peer's Ed25519 root — no truncation.
pub fn signed_prekey(peer: &IdentityRoot) -> Label {
    Label::new_validated(format!("prekey.spk.{}", peer.to_hex()))
}

/// PQ prekey (one-time) for a specific peer.
pub fn pq_prekey(peer: &IdentityRoot) -> Label {
    Label::new_validated(format!("prekey.pqpk.{}", peer.to_hex()))
}

/// PQ last-resort prekey for a specific peer.
pub fn pq_prekey_last_resort(peer: &IdentityRoot) -> Label {
    Label::new_validated(format!("prekey.pqpk-lr.{}", peer.to_hex()))
}

// ── Locator keypairs ────────────────────────────────────────────────

/// Profile DHT record keypair.
pub fn locator_profile() -> Label {
    Label::new_validated("locator.profile".into())
}

/// Friend inbox DHT record keypair.
pub fn locator_inbox() -> Label {
    Label::new_validated("locator.inbox".into())
}

/// Mailbox DHT record keypair.
pub fn locator_mailbox() -> Label {
    Label::new_validated("locator.mailbox".into())
}

/// Friend list DHT record keypair.
pub fn locator_friend_list() -> Label {
    Label::new_validated("locator.friend-list".into())
}

// ── DM log keypairs (per-session, keyed by SessionAnchor) ───────────

/// DM DhtLog keypair for a peer-to-peer session.
pub fn dm_log_keypair(anchor: &SessionAnchor) -> Label {
    Label::new_validated(format!("log.dm.{}", anchor.to_hex()))
}

// ── Channel log keypairs (per-community-per-channel) ────────────────

/// Channel DhtLog keypair. `channel_uuid` is the 16-byte channel UUID
/// encoded as 32 hex chars.
pub fn channel_log_keypair(gov: &GovernanceKey, channel_uuid: &[u8; 16]) -> Label {
    Label::new_validated(format!(
        "log.channel.{}.{}",
        hex::encode(gov.canonical_bytes()),
        hex::encode(channel_uuid),
    ))
}

// ── Community governance ────────────────────────────────────────────

/// Governance record keypair.
pub fn community_governance(gov: &GovernanceKey) -> Label {
    Label::new_validated(format!(
        "community.governance.{}",
        hex::encode(gov.canonical_bytes()),
    ))
}

/// Registry record keypair.
pub fn community_registry(gov: &GovernanceKey) -> Label {
    Label::new_validated(format!(
        "community.registry.{}",
        hex::encode(gov.canonical_bytes()),
    ))
}

/// Per-member slot derivation seed.
pub fn community_slot(gov: &GovernanceKey, slot_index: u32) -> Label {
    Label::new_validated(format!(
        "community.slot.{}.{}",
        hex::encode(gov.canonical_bytes()),
        slot_index,
    ))
}

/// Persona (pseudonym) seed for a community.
pub fn persona_seed(gov: &GovernanceKey) -> Label {
    Label::new_validated(format!(
        "persona.seed.{}",
        hex::encode(gov.canonical_bytes()),
    ))
}

// ── Session ratchet state ───────────────────────────────────────────

/// Triple Ratchet session state.
pub fn session_state(anchor: &SessionAnchor) -> Label {
    Label::new_validated(format!("session.ratchet.{}", anchor.to_hex()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::originate_from_seed;
    use crate::origin::seed::OriginSeed;
    use crate::session::session_anchor;
    use zeroize::Zeroizing;

    fn test_root(byte: u8) -> IdentityRoot {
        originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([byte; 32]))
        ).unwrap().root
    }

    fn test_gov() -> GovernanceKey {
        GovernanceKey::parse("VLD0:TestGovernanceKey123").unwrap()
    }

    fn test_anchor() -> SessionAnchor {
        let a = test_root(0x01);
        let b = test_root(0x02);
        session_anchor(&a, &b).unwrap()
    }

    #[test]
    fn origin_labels_valid() {
        let _ = origin_seed();
        let _ = dh_seed();
        let _ = revocation_certificate();
    }

    #[test]
    fn prekey_labels_use_full_hex() {
        let root = test_root(0x01);
        let label = signed_prekey(&root);
        let hex_part = label.as_str().strip_prefix("prekey.spk.").unwrap();
        assert_eq!(hex_part.len(), 64, "must use full 64 hex chars, not truncated");
        assert_eq!(hex_part, root.to_hex());
    }

    #[test]
    fn prekey_labels_distinct_peers() {
        let a = test_root(0x01);
        let b = test_root(0x02);
        assert_ne!(signed_prekey(&a), signed_prekey(&b));
        assert_ne!(pq_prekey(&a), pq_prekey(&b));
        assert_ne!(pq_prekey_last_resort(&a), pq_prekey_last_resort(&b));
    }

    #[test]
    fn prekey_labels_same_peer_deterministic() {
        let root = test_root(0x01);
        assert_eq!(signed_prekey(&root), signed_prekey(&root));
    }

    #[test]
    fn collision_that_old_truncation_would_cause() {
        // Two roots that share the first 6 bytes (12 hex chars) but
        // differ at byte 7. Under the old [..12] truncation, their
        // labels would collide. Under full-width, they do not.
        //
        // We can't easily construct such roots deterministically
        // (Ed25519 key derivation is a hash), so we test the structural
        // property: two different roots always produce different labels.
        let a = test_root(0x01);
        let b = test_root(0x02);
        assert_ne!(
            signed_prekey(&a).as_str(),
            signed_prekey(&b).as_str(),
            "different roots MUST produce different labels"
        );
    }

    #[test]
    fn governance_labels_use_canonical_bytes() {
        let gov = test_gov();
        let label = community_governance(&gov);
        // The label must contain the hex of canonical_bytes, not the
        // VLD0: prefix or the display form.
        let canonical_hex = hex::encode(gov.canonical_bytes());
        assert!(
            label.as_str().contains(&canonical_hex),
            "label must contain canonical hex: {}", label.as_str()
        );
        assert!(
            !label.as_str().contains("VLD0"),
            "label must NOT contain substrate prefix"
        );
    }

    #[test]
    fn dm_log_label_uses_anchor() {
        let anchor = test_anchor();
        let label = dm_log_keypair(&anchor);
        assert!(label.as_str().starts_with("log.dm."));
        assert_eq!(
            label.as_str().strip_prefix("log.dm.").unwrap().len(),
            64,
            "must use full 64 hex chars of anchor"
        );
    }

    #[test]
    fn channel_log_label_includes_channel_uuid() {
        let gov = test_gov();
        let channel = [0xAA; 16];
        let label = channel_log_keypair(&gov, &channel);
        assert!(label.as_str().starts_with("log.channel."));
        assert!(label.as_str().contains(&hex::encode(channel)));
    }

    #[test]
    fn session_state_label() {
        let anchor = test_anchor();
        let label = session_state(&anchor);
        assert!(label.as_str().starts_with("session.ratchet."));
    }

    #[test]
    fn all_labels_pass_grammar() {
        // Every builder must produce labels that pass the grammar.
        // This is inherently tested by new_validated (panics on failure),
        // but we exercise every builder path explicitly.
        let root = test_root(0x01);
        let gov = test_gov();
        let anchor = test_anchor();

        let labels = [
            origin_seed(),
            dh_seed(),
            revocation_certificate(),
            signed_prekey(&root),
            pq_prekey(&root),
            pq_prekey_last_resort(&root),
            locator_profile(),
            locator_inbox(),
            locator_mailbox(),
            locator_friend_list(),
            dm_log_keypair(&anchor),
            channel_log_keypair(&gov, &[0; 16]),
            community_governance(&gov),
            community_registry(&gov),
            community_slot(&gov, 0),
            community_slot(&gov, 255),
            persona_seed(&gov),
            session_state(&anchor),
        ];

        for label in &labels {
            // Grammar check: lowercase alphanumeric, dots, hyphens
            assert!(
                label.as_str().bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'
                }),
                "label violates grammar: {}", label.as_str()
            );
            assert!(!label.as_str().is_empty());
            assert!(label.as_str().len() <= 256);
        }
    }

    #[test]
    fn no_label_contains_colon() {
        // VLD0: prefix must never leak into a label
        let gov = GovernanceKey::parse("VLD0:SomeGovernanceKey").unwrap();
        let labels = [
            community_governance(&gov),
            community_registry(&gov),
            community_slot(&gov, 42),
            persona_seed(&gov),
        ];
        for label in &labels {
            assert!(
                !label.as_str().contains(':'),
                "label must not contain colon: {}", label.as_str()
            );
        }
    }
}
