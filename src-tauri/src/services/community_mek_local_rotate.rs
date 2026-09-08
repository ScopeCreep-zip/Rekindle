//! Operator-requested community MEK rotation.
//!
//! Now a thin shell over
//! [`rekindle_mek_rotation::rotate_mek_on_request`], which both tracks
//! drive. What it replaced was the last MEK-vault writer in the
//! workspace, and it was wrong in three independent ways:
//!
//! * It published wrapped copies to the registry's MEK-vault subkey — a
//!   write `o_cnt: 0` grants nobody a credential for, of a key
//!   `communities-channels.md` says is *"**never** written to DHT"*.
//! * It read the v1.0 member index to enumerate recipients.
//! * It refused to run without a `registry_owner_keypair`, making
//!   rotation a creator privilege. That is the coordinator in
//!   miniature.
//!
//! Delivery is now per-recipient `app_call` over the gossip overlay's
//! online set, the same path a departure rotation takes.

use std::sync::Arc;

use crate::state::SharedState;

pub async fn rotate_mek_local(
    app_handle: &tauri::AppHandle,
    state: &SharedState,
    community_id: &str,
) -> Result<(), String> {
    let pool = tauri::Manager::try_state::<crate::db::DbPool>(app_handle)
        .ok_or_else(|| "DbPool state missing".to_string())?
        .inner()
        .clone();
    let adapter =
        crate::services::mek_adapter::MekAdapter::new(Arc::clone(state), app_handle.clone(), pool);

    // `None` = the community-wide key. The adapter's `MekPersist` impl
    // writes it to the keystore, so the caller no longer threads a
    // `KeystoreHandle` in just for that.
    rekindle_mek_rotation::rotate_mek_on_request(adapter.as_ref(), community_id, None)
        .await
        .map_err(|e| e.to_string())
}
