//! Ownership transfer and the encrypted community backup it writes.

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;

use crate::daemon::dispatch::{adapter, state_error, DaemonContext};

/// Give another member administrative control.
///
/// Named `TransferOwnership` on the wire, but under flat governance
/// there is no owner to move: `GovernanceState.creator` is fixed by the
/// genesis entries and holds `ALL` permanently. See
/// `rekindle_governance_runtime::ownership` for why that is deliberate
/// and what Matrix concluded about the same question.
///
/// This used to rewrite `owner_pseudonym` / `operator_pseudonyms` in
/// the v1.0 governance-manifest metadata subkey. Nothing reads those
/// under v2.0 — permissions come from the merged CRDT — so it reported
/// success and granted the new owner nothing at all.
pub(crate) async fn handle_transfer_ownership(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
    new_owner_pseudonym: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.resolve_community(governance_key) {
        return e;
    }

    // Permission comes from the CRDT, not from `is_operator` — that
    // flag is v1.0 residue and under `o_cnt: 0` grants nothing.
    match rekindle_governance_runtime::ownership::grant_administration(
        &adapter(ctx),
        governance_key,
        new_owner_pseudonym,
    )
    .await
    {
        Ok(outcome) => IpcResponse::ok(&serde_json::json!({
            "granted": new_owner_pseudonym,
            "roleId": outcome.role_id,
            "relinquished": outcome.relinquished,
            // The caller needs to know: a creator cannot step down, so
            // "transfer" from the creator is a grant and nothing more.
            "stillCreator": outcome.still_creator,
        })),
        Err(e) => IpcResponse::error(500, format!("grant administration failed: {e}")),
    }
}

pub(crate) fn write_encrypted_backup(
    path: &std::path::Path,
    data: &[u8],
    key: &[u8; 32],
) -> anyhow::Result<()> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow::anyhow!("AES init: {e}"))?;
    let mut nonce_bytes = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    std::fs::write(path, &output)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
