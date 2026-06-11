//! Canonical key label constants and builder functions.
//!
//! Hierarchical dot-separated naming: `{domain}.{category}.{identifier}`.
//! Labels are validated on every store/load — injection-safe by construction.

use crate::error::{StorageError, StorageResult};

// ── Identity ────────────────────────────────────────────────────────

pub const SIGNING_KEY: &str = "identity.signing-key";
pub const IDENTITY_MASTER_SEED: &str = "identity.master-seed";
pub const IDENTITY_ED25519_SEED: &str = "identity.ed25519-seed";
pub const IDENTITY_X25519_SEED: &str = "identity.x25519-seed";

// ── Signal prekeys ──────────────────────────────────────────────────

pub fn signed_prekey(id: u64) -> String {
    assert_valid(&format!("signal.spk.{id}"))
}

pub fn one_time_prekey(id: u64) -> String {
    assert_valid(&format!("signal.otpk.{id}"))
}

pub fn pq_prekey(id: u64) -> String {
    assert_valid(&format!("signal.pqpk.{id}"))
}

pub fn pq_last_resort() -> String {
    "signal.pqpk-lr".to_string()
}

/// Per-target signed prekey (stored during friend request send, read during accept).
pub fn target_signed_prekey(target_short: &str) -> String {
    assert_valid(&format!("signal.spk.{}", strip_key_prefix(target_short)))
}

/// Per-target PQ prekey (one-time, stored during friend request send).
pub fn target_pq_prekey(target_short: &str) -> String {
    assert_valid(&format!("signal.pqpk.{}", strip_key_prefix(target_short)))
}

/// Per-target PQ last-resort prekey (stored during friend request send).
pub fn target_pq_last_resort(target_short: &str) -> String {
    assert_valid(&format!("signal.pqpk-lr.{}", strip_key_prefix(target_short)))
}

// ── DhtLog keypairs ─────────────────────────────────────────────────

pub fn dm_log_keypair(log_key_short: &str) -> String {
    assert_valid(&format!("dht.dm-log.{}", strip_key_prefix(log_key_short)))
}

pub fn channel_log_keypair(log_key_short: &str) -> String {
    assert_valid(&format!("dht.channel-log.{}", strip_key_prefix(log_key_short)))
}

// ── DHT record keypairs ─────────────────────────────────────────────

pub const PROFILE_KEYPAIR: &str = "dht.profile";
pub const FRIEND_LIST_KEYPAIR: &str = "dht.friend-list";
pub const FRIEND_INBOX_KEYPAIR: &str = "dht.friend-inbox";

// ── Community governance ────────────────────────────────────────────

pub fn governance_keypair(gov_key_short: &str) -> String {
    assert_valid(&format!("community.governance.{}", strip_key_prefix(gov_key_short)))
}

pub fn registry_keypair(reg_key_short: &str) -> String {
    assert_valid(&format!("community.registry.{}", strip_key_prefix(reg_key_short)))
}

pub fn slot_seed(gov_key_short: &str, slot_index: u32) -> String {
    assert_valid(&format!("community.slot.{}.{slot_index}", strip_key_prefix(gov_key_short)))
}

/// Strip the `VLD0:` (or any `XXX:`) type-tag prefix from a key string.
/// Key labels allow only alphanumeric, dots, and hyphens — the colon in
/// the prefix would fail validation. The prefix is a type tag, not part
/// of the key identifier.
pub fn strip_key_prefix(key: &str) -> &str {
    key.find(':').map_or(key, |pos| &key[pos + 1..])
}

/// Assert a label is valid at construction time. Every label builder
/// calls this before returning. A malformed label panics here — not
/// later in a vault store call with an opaque "key label invalid" error.
fn assert_valid(label: &str) -> String {
    if let Err(e) = validate(label) {
        panic!(
            "BUG: label builder produced invalid label '{label}': {e}. \
             This is a code defect — the builder must strip prefixes and \
             restrict characters before constructing the label."
        );
    }
    label.to_string()
}

// ── Validation ──────────────────────────────────────────────────────

/// Labels must be 1–128 chars of ASCII alphanumeric, dots, and hyphens.
pub fn validate(label: &str) -> StorageResult<()> {
    if label.is_empty() || label.len() > 128 {
        return Err(StorageError::KeyLabelInvalid {
            label: label.to_string(),
        });
    }
    if !label
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return Err(StorageError::KeyLabelInvalid {
            label: label.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_labels_pass() {
        assert!(validate(SIGNING_KEY).is_ok());
        assert!(validate(&signed_prekey(1)).is_ok());
        assert!(validate(&dm_log_keypair("abc123")).is_ok());
        assert!(validate(&governance_keypair("def456")).is_ok());
    }

    #[test]
    fn vld0_prefix_stripped_from_label_builders() {
        // Veilid keys arrive as "VLD0:5fVBSx3..." — the colon would
        // fail validation. strip_key_prefix removes the type-tag prefix.
        let gov = governance_keypair("VLD0:5fVBSx3abc");
        assert!(validate(&gov).is_ok(), "governance label with VLD0: prefix must validate: {gov}");
        assert!(!gov.contains(':'), "colon must not appear in label: {gov}");

        let reg = registry_keypair("VLD0:NC-OBPrd123");
        assert!(validate(&reg).is_ok(), "registry label with VLD0: prefix must validate: {reg}");

        let dm = dm_log_keypair("VLD0:h1-4E4ua1O4");
        assert!(validate(&dm).is_ok(), "dm_log label with VLD0: prefix must validate: {dm}");

        let ch = channel_log_keypair("VLD0:FZpSzBXzZ7");
        assert!(validate(&ch).is_ok(), "channel_log label with VLD0: prefix must validate: {ch}");
    }

    #[test]
    fn plain_keys_pass_through_strip_unchanged() {
        // Keys without a colon prefix pass through strip_key_prefix unchanged.
        assert_eq!(strip_key_prefix("abc123"), "abc123");
        assert_eq!(strip_key_prefix("5fVBSx3abc"), "5fVBSx3abc");
    }

    #[test]
    fn colon_prefix_stripped_correctly() {
        assert_eq!(strip_key_prefix("VLD0:5fVBSx3abc"), "5fVBSx3abc");
        assert_eq!(strip_key_prefix("VLD1:xyz"), "xyz");
        assert_eq!(strip_key_prefix(":empty-prefix"), "empty-prefix");
    }

    #[test]
    fn empty_label_rejected() {
        assert!(validate("").is_err());
    }

    #[test]
    fn slash_rejected() {
        assert!(validate("foo/bar").is_err());
    }

    #[test]
    fn space_rejected() {
        assert!(validate("foo bar").is_err());
    }

    #[test]
    fn overlength_rejected() {
        let long = "a".repeat(129);
        assert!(validate(&long).is_err());
    }
}
