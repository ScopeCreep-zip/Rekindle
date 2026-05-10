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
    format!("signal.spk.{id}")
}

pub fn one_time_prekey(id: u64) -> String {
    format!("signal.otpk.{id}")
}

pub fn pq_prekey(id: u64) -> String {
    format!("signal.pqpk.{id}")
}

pub fn pq_last_resort() -> String {
    "signal.pqpk-lr".to_string()
}

// ── DhtLog keypairs ─────────────────────────────────────────────────

pub fn dm_log_keypair(log_key_short: &str) -> String {
    format!("dht.dm-log.{log_key_short}")
}

pub fn channel_log_keypair(log_key_short: &str) -> String {
    format!("dht.channel-log.{log_key_short}")
}

// ── DHT record keypairs ─────────────────────────────────────────────

pub const PROFILE_KEYPAIR: &str = "dht.profile";
pub const FRIEND_LIST_KEYPAIR: &str = "dht.friend-list";
pub const FRIEND_INBOX_KEYPAIR: &str = "dht.friend-inbox";

// ── Community governance ────────────────────────────────────────────

pub fn governance_keypair(gov_key_short: &str) -> String {
    format!("community.governance.{gov_key_short}")
}

pub fn registry_keypair(reg_key_short: &str) -> String {
    format!("community.registry.{reg_key_short}")
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
