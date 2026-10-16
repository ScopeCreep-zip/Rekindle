//! The bus clients an identity trusts (plan D1): each client's static
//! Noise public key and its clearance, in the identity's vault
//! (`VaultKey::ClientKey`). The client keeps its private key in its own file
//! (`rekindle_ipc::noise_keys`, `DataRoot.config/agent-keys`); frontends
//! never link the vault.
//!
//! A successful unlock registers the unlocking client at `Admin` (the
//! passphrase is the proof). While unlocked, the identity's clients are in
//! the bus server's registry, which stamps every frame's clearance from it,
//! so a lock lowers their live connections at once.

use rekindle_ipc::message::{AgentType, SecurityLevel};
use rekindle_vault::VaultKey;

use super::dispatch::DaemonContext;

/// The clearance a registered client holds, as the vault stores it.
const ADMIN: &[u8] = b"admin";

/// The registry name of a user client.
fn client_name(key_hex: &str) -> String {
    format!("client-{}", rekindle_utils::text::prefix(key_hex, 16))
}

/// Register `key` with the unlocked identity at `Admin`: in its vault, then
/// in the registry (its live connection is raised with its next frame).
///
/// # Errors
/// The vault is locked or its write failed.
pub async fn register_unlocking_client(ctx: &DaemonContext, key: &[u8; 32]) -> Result<(), String> {
    let key_hex = hex::encode(key);
    {
        let guard = ctx.vault.lock();
        let vault = guard.as_ref().ok_or("no identity is unlocked")?;
        vault
            .put(
                &VaultKey::ClientKey {
                    public_key: key_hex.clone(),
                },
                ADMIN,
            )
            .map_err(|e| format!("client key: {e}"))?;
    }
    install(ctx, &key_hex).await;
    Ok(())
}

/// Put every client the unlocked identity trusts into the registry.
pub async fn load(ctx: &DaemonContext) {
    let keys: Vec<String> = {
        let guard = ctx.vault.lock();
        let Some(vault) = guard.as_ref() else {
            return;
        };
        match vault.list("clients") {
            Ok(keys) => keys
                .into_iter()
                .filter_map(|key| match key {
                    VaultKey::ClientKey { public_key } => {
                        let level = vault.get(&VaultKey::ClientKey {
                            public_key: public_key.clone(),
                        });
                        match level {
                            Ok(Some(value)) if value.as_slice() == ADMIN => Some(public_key),
                            _ => {
                                tracing::warn!(key = %public_key, "a client key of unknown clearance is not loaded");
                                None
                            }
                        }
                    }
                    _ => None,
                })
                .collect(),
            Err(e) => {
                tracing::error!(error = %e, "the identity's clients cannot be listed");
                return;
            }
        }
    };
    for key_hex in keys {
        install(ctx, &key_hex).await;
    }
}

/// Register one client key at `Admin`, unless the registry already knows
/// the key (an agent keeps its own registration).
async fn install(ctx: &DaemonContext, key_hex: &str) {
    let Ok(bytes) = hex::decode(key_hex) else {
        tracing::warn!(key = %key_hex, "a stored client key is not hex");
        return;
    };
    let Ok(key) = <[u8; 32]>::try_from(bytes) else {
        tracing::warn!(key = %key_hex, "a stored client key is not 32 bytes");
        return;
    };
    let name = client_name(key_hex);
    let mut registry = ctx.registry.write().await;
    if registry.lookup(&key).is_some() {
        return;
    }
    registry.register(
        name.clone(),
        key,
        SecurityLevel::Admin,
        AgentType::Human,
        Vec::new(),
    );
    drop(registry);
    ctx.unlocked_clients.lock().push(name);
}

/// Take the identity's clients out of the registry (lock).
pub async fn unload(ctx: &DaemonContext) {
    let names = std::mem::take(&mut *ctx.unlocked_clients.lock());
    let mut registry = ctx.registry.write().await;
    for name in names {
        registry.revoke_by_name(&name);
    }
}
