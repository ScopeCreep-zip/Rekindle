//! The shell's effects of a governance projection (plan E3.1 B2c): the Host
//! projected merged governance onto the community store
//! (`rekindle_node::host::adapters::governance::projection`), and these are
//! what it cannot run yet. `ShellHooks::on_governance_projected` names the
//! owner step of each.

use rekindle_node::host::adapters::governance::projection::GovernanceProjected;

use crate::state::SharedState;

/// Run the shell's effects of a projection of `community_id`.
pub(super) fn on_projected(
    state: &SharedState,
    community_id: &str,
    projected: &GovernanceProjected,
) {
    let scope = crate::state_helpers::login_scope_or_closed(state);
    // New Plate Gate segment records opened (the watch path, E3.1 C1).
    let (s, c) = (state.clone(), community_id.to_string());
    scope.spawn_or_drop("open new plate segments", async move {
        crate::services::community::segments::open_new_segments(&s, &c).await;
    });
    // The watches refreshed when the merge named a new record (E3.1 C1).
    if projected.new_watch_targets {
        let (s, c) = (state.clone(), community_id.to_string());
        scope.spawn_or_drop("governance watch refresh", async move {
            if let Err(e) = crate::services::community::watch::watch_community_records(&s, &c).await
            {
                tracing::warn!(community = %c, error = %e, "watches not refreshed after a governance update");
            }
        });
    }
}

/// One presence poll right after a slot grant (presence, E3.4).
pub(super) fn spawn_presence_poll_tick(state: &SharedState, community_id: &str) {
    let (s, c) = (state.clone(), community_id.to_string());
    crate::state_helpers::login_scope_or_closed(state).spawn_or_drop("presence poll tick", async move {
        if let Err(e) = crate::services::community::presence_poll_tick_public(&s, &c).await {
            tracing::debug!(error = %e, community = %c, "presence poll after a slot grant failed");
        }
    });
}

/// A key the Host's cache took, into the shell's own cache (MEK, E3.5).
pub(super) fn install_mek(
    state: &SharedState,
    community_id: &str,
    scope: rekindle_types::channel_keys::KeyScope,
    key_bytes: [u8; 32],
    generation: u64,
) {
    let key =
        rekindle_crypto::group::media_key::MediaEncryptionKey::from_bytes(key_bytes, generation);
    if !crate::state_helpers::install_mek(state, community_id, scope, key) {
        tracing::debug!(community = %community_id, %scope, "the shell kept a newer key");
    }
}

/// Flip `onboarding_complete[community_id]` on the personal sync ReadState
/// (architecture §28.4; cross-device sync, E5.2).
pub(super) fn push_onboarding_complete_to_sync(state: &SharedState, community_id: &str) {
    let Ok(pool) = state.db.current() else {
        return;
    };
    let (s, c) = (state.clone(), community_id.to_string());
    crate::state_helpers::login_scope_or_closed(state).spawn_or_drop("onboarding sync push", async move {
        if let Err(e) = push_onboarding_complete(&s, &pool, &c).await {
            tracing::warn!(community = %c, error = %e, "onboarding completion not pushed to personal sync");
        }
    });
}

async fn push_onboarding_complete(
    state: &SharedState,
    pool: &rekindle_db::Db,
    community_id: &str,
) -> Result<(), String> {
    use crate::services::cross_device_sync::{
        open_personal_sync_record, read_read_state, write_read_state,
    };
    let Some(handle) = open_personal_sync_record(state, pool).await else {
        return Ok(());
    };
    let master_secret = state
        .identity_secret
        .lock()
        .as_ref()
        .copied()
        .ok_or_else(|| "identity secret not available".to_string())?;
    let sync_key = rekindle_secrets::sync_key::SyncKey::from_master_secret(&master_secret);
    let mut doc = read_read_state(state, &handle, &sync_key)
        .await
        .unwrap_or_default();
    doc.onboarding_complete
        .insert(community_id.to_string(), true);
    write_read_state(state, &handle, &sync_key, doc).await?;
    Ok(())
}
