//! Ownership transfer and the encrypted community backup it writes.

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;

use crate::daemon::dispatch::{state_error, DaemonContext};

pub(crate) async fn handle_transfer_ownership(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
    new_owner_pseudonym: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(governance_key) {
        Ok(m) => m,
        Err(e) => return e,
    };
    if !membership.is_operator {
        return IpcResponse::error(403, "not an operator for this community");
    }

    let dht = match transport.dht() {
        Ok(d) => d,
        Err(e) => return IpcResponse::error(500, format!("DHT: {e}")),
    };

    // Read and update governance metadata
    let Ok(Some(metadata)) = dht
        .governance()
        .read_metadata(&membership.governance_key)
        .await
    else {
        return IpcResponse::error(500, "cannot read governance metadata");
    };

    let mut updated = metadata;
    let old_owner = updated.owner_pseudonym.clone();
    updated.owner_pseudonym = new_owner_pseudonym.to_string();
    updated.operator_pseudonyms.retain(|p| p != &old_owner);
    if !updated
        .operator_pseudonyms
        .contains(&new_owner_pseudonym.to_string())
    {
        updated
            .operator_pseudonyms
            .push(new_owner_pseudonym.to_string());
    }

    if let Err(e) = dht
        .governance()
        .write_metadata(&membership.governance_key, &updated)
        .await
    {
        return IpcResponse::error(500, format!("metadata update failed: {e}"));
    }

    // Update local session: current user is no longer operator
    {
        let mut guard = ctx.session.write();
        if let Some(ref mut s) = *guard {
            if let Some(m) = s.communities.get_mut(governance_key) {
                m.is_operator = false;
                m.governance_keypair_label = None;
            }
        }
    }
    if let Err(e) = ctx.save_session() {
        return e;
    }

    IpcResponse::ok(&serde_json::json!({
        "transferred": true,
        "old_owner": old_owner,
        "new_owner": new_owner_pseudonym,
    }))
}

pub(super) fn write_encrypted_backup(
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
