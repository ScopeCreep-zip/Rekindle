//! The identity's record owner entries (key and keypair), in the unlocked
//! vault (plan D1; audit 2026-10-20 D-5).

use rekindle_vault::typed::owner_keys::{self, OwnerEntry, OwnerKey};

use crate::state::AppState;

/// Store `record`'s key and owner keypair in the unlocked vault.
///
/// # Errors
/// No vault is unlocked, or the write failed.
pub fn persist_owner(state: &AppState, record: OwnerKey, entry: &OwnerEntry) -> Result<(), String> {
    let vault = state.keystore.lock();
    let vault = vault.as_ref().ok_or("vault is locked")?;
    owner_keys::persist_owner(vault, record, entry)
        .map_err(|e| format!("store {record:?} owner keypair: {e}"))
}

/// `record`'s entry from the unlocked vault, if one is stored.
///
/// # Errors
/// No vault is unlocked, or the read failed.
pub fn load_owner(state: &AppState, record: OwnerKey) -> Result<Option<OwnerEntry>, String> {
    let vault = state.keystore.lock();
    let vault = vault.as_ref().ok_or("vault is locked")?;
    owner_keys::load_owner(vault, record).map_err(|e| format!("read {record:?} owner keypair: {e}"))
}
