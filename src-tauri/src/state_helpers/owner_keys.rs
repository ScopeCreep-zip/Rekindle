//! The identity's record owner keypairs, in the unlocked vault (plan D1).

use rekindle_vault::typed::owner_keys::{self, OwnerKey};

use crate::state::AppState;

/// Store `record`'s owner keypair in the unlocked vault.
///
/// # Errors
/// No vault is unlocked, or the write failed.
pub fn persist_owner_keypair(
    state: &AppState,
    record: OwnerKey,
    keypair: &str,
) -> Result<(), String> {
    let vault = state.keystore.lock();
    let vault = vault.as_ref().ok_or("vault is locked")?;
    owner_keys::persist_owner_keypair(vault, record, keypair)
        .map_err(|e| format!("store {record:?} owner keypair: {e}"))
}

/// `record`'s owner keypair from the unlocked vault, if one is stored.
///
/// # Errors
/// No vault is unlocked, or the read failed.
pub fn load_owner_keypair(state: &AppState, record: OwnerKey) -> Result<Option<String>, String> {
    let vault = state.keystore.lock();
    let vault = vault.as_ref().ok_or("vault is locked")?;
    owner_keys::load_owner_keypair(vault, record)
        .map_err(|e| format!("read {record:?} owner keypair: {e}"))
}
