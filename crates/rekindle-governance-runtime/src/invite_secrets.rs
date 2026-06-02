//! Invite-secrets DHT record — modular indirection for invite payloads.
//!
//! An invite's encrypted `InviteSecrets` blob is several KB (route blob,
//! channel keys, MEK, slot seed) — far larger than the ~4 KB per-subkey cap
//! of the 255-slot governance SMPL record. Storing it inline in a
//! `GovernanceEntry::InviteCreated` subkey overflows schema validation, so
//! the blob lives in its own single-owner DFLT(1) record and governance
//! carries only the pointer (`secrets_record_key`). This mirrors the
//! pointer-indirection used by IPNS / Session / Matrix and the architecture's
//! own DFLT pointer records (§6, §line 180).

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;

/// Create the invite-secrets DFLT record and write `encrypted_b64` into its
/// single owner subkey. Returns the record key for embedding in the
/// governance `InviteCreated` entry.
pub async fn publish_invite_secrets<D: GovernanceRuntimeDeps>(
    deps: &D,
    encrypted_b64: &str,
) -> Result<String, GovernanceRuntimeError> {
    let record = deps.create_dflt_record().await?;
    let owner = record.owner_keypair.clone().ok_or_else(|| {
        GovernanceRuntimeError::Adapter("invite-secrets DFLT record missing owner keypair".into())
    })?;
    let stale = deps
        .set_dht_value(
            &record.record_key,
            0,
            encrypted_b64.as_bytes().to_vec(),
            Some(owner),
        )
        .await?;
    if stale.is_some() {
        return Err(GovernanceRuntimeError::Adapter(
            "invite-secrets write was not accepted by the network".into(),
        ));
    }
    Ok(record.record_key)
}

/// Open (read-only) and read the encrypted `InviteSecrets` blob from the
/// invite-secrets record pointed to by a governance `InviteCreated` entry.
pub async fn fetch_invite_secrets<D: GovernanceRuntimeDeps>(
    deps: &D,
    record_key: &str,
) -> Result<String, GovernanceRuntimeError> {
    deps.open_dht_record(record_key, None).await?;
    let bytes = deps
        .get_dht_value(record_key, 0, true)
        .await?
        .filter(|b| !b.is_empty())
        .ok_or_else(|| GovernanceRuntimeError::Adapter("invite-secrets record is empty".into()))?;
    String::from_utf8(bytes)
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("invite-secrets not UTF-8: {e}")))
}
