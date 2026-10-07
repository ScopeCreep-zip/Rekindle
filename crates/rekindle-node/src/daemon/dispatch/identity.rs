//! Identity dispatch handlers: Create, Show, Export, Rotate, Destroy, Wipe.

use zeroize::Zeroize;

use rekindle_transport::operations::identity;
use rekindle_transport::session::{Session, SessionIdentity};

use crate::daemon::DaemonState;
use rekindle_ipc::protocol::IpcResponse;

use crate::daemon::shutdown::ExitReason;

use super::{state_error, teardown_unlocked, transition, DaemonContext};

/// Handle IdentityCreate — full ceremony, daemon-side.
pub(crate) async fn handle_create(
    ctx: &DaemonContext,
    state: DaemonState,
    display_name: &str,
) -> IpcResponse {
    // Identity creation can happen in Locked state (no existing identity).
    if !matches!(state, DaemonState::Locked | DaemonState::Operational) {
        return state_error(state, "identity creation");
    }

    // Check not already initialized
    if ctx.session.read().is_some() {
        return IpcResponse::error(409, "identity already exists — destroy it first");
    }

    let display_name = display_name.trim().to_owned();

    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };

    // In-memory Signal stores for the ceremony
    let prekey_store = Box::new(rekindle_transport::crypto::signal_store::MemoryPreKeyStore::new());
    let session_store =
        Box::new(rekindle_transport::crypto::signal_store::MemorySessionStore::new());

    // The ceremony creates our records in a record pool of its own, ended
    // with it: they are re-opened by the unlock that follows (plan C7.4).
    if let Err(e) = transport.start_records() {
        return IpcResponse::error(503, format!("record pool: {e}"));
    }
    let created = identity::create_identity(
        &transport,
        &display_name,
        "Hello from Rekindle!",
        prekey_store,
        session_store,
    )
    .await;
    transport.end_records();
    let mut created = match created {
        Ok(c) => c,
        Err(e) => return IpcResponse::error(500, format!("identity creation failed: {e}")),
    };

    // Store signing key in OS keyring
    if let Err(e) = crate::state::keystore::store_signing_key(&created.signing_key_bytes).await {
        return IpcResponse::error(500, format!("failed to store signing key: {e}"));
    }

    // Store keypair bytes
    if let Err(e) =
        crate::state::keystore::store_keypair_bytes("profile", &created.profile_keypair_bytes).await
    {
        return IpcResponse::error(500, format!("failed to store profile keypair: {e}"));
    }
    if let Err(e) = crate::state::keystore::store_keypair_bytes(
        "friend_list",
        &created.friend_list_keypair_bytes,
    )
    .await
    {
        return IpcResponse::error(500, format!("failed to store friend list keypair: {e}"));
    }

    // Store prekey material
    let (spk_id, ref spk_bytes) = created.prekey_material.signed_prekey;
    if let Err(e) =
        crate::state::keystore::store_keypair_bytes(&format!("signed-prekey-{spk_id}"), spk_bytes)
            .await
    {
        return IpcResponse::error(500, format!("failed to store signed prekey: {e}"));
    }
    for (otpk_id, ref otpk_bytes) in &created.prekey_material.one_time_prekeys {
        if let Err(e) = crate::state::keystore::store_keypair_bytes(
            &format!("one-time-prekey-{otpk_id}"),
            otpk_bytes,
        )
        .await
        {
            return IpcResponse::error(500, format!("failed to store one-time prekey: {e}"));
        }
    }

    // Zeroize signing key bytes after keyring storage
    created.signing_key_bytes.zeroize();

    // Build and persist session
    let session = Session::new(SessionIdentity {
        public_key_hex: created.public_key_hex.clone(),
        display_name: display_name.clone(),
        profile_dht_key: created.profile_dht_key.clone(),
        mailbox_dht_key: created.mailbox_dht_key.clone(),
        friend_list_dht_key: created.friend_list_dht_key.clone(),
        friend_inbox_key: created.friend_inbox_key.clone(),
        friend_inbox_keypair_hex: created.friend_inbox_keypair_hex.clone(),
        profile_keypair_bytes: None,
        friend_list_keypair_bytes: None,
    });

    *ctx.session.write() = Some(session);
    if let Err(e) = ctx.save_session() {
        return e;
    }

    IpcResponse::ok(&serde_json::json!({
        "status": "created",
        "public_key": created.public_key_hex,
        "display_name": display_name,
        "profile_dht_key": created.profile_dht_key,
        "mailbox_dht_key": created.mailbox_dht_key,
        "friend_list_dht_key": created.friend_list_dht_key,
    }))
}

/// Handle IdentityShow — display identity info.
///
/// Available in ANY state where a session is loaded (including Locked).
/// Identity metadata (public key, display name, DHT keys) is not protected
/// by the signing key — it's safe to return without unlocking.
pub(crate) fn handle_show(ctx: &DaemonContext, _state: DaemonState) -> IpcResponse {
    ctx.require_session(|session| {
        IpcResponse::ok(&serde_json::json!({
            "public_key": session.identity.public_key_hex,
            "display_name": session.identity.display_name,
            "profile_dht_key": session.identity.profile_dht_key,
            "mailbox_dht_key": session.identity.mailbox_dht_key,
            "friend_list_dht_key": session.identity.friend_list_dht_key,
            "communities": session.communities.len(),
        }))
    })
    .unwrap_or_else(|e| e)
}

/// Handle IdentityExport — return identity metadata for client-side file write.
pub(crate) fn handle_export(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    ctx.require_session(|session| {
        IpcResponse::ok(&serde_json::json!({
            "public_key": session.identity.public_key_hex,
            "display_name": session.identity.display_name,
            "profile_dht_key": session.identity.profile_dht_key,
            "mailbox_dht_key": session.identity.mailbox_dht_key,
            "friend_list_dht_key": session.identity.friend_list_dht_key,
        }))
    })
    .unwrap_or_else(|e| e)
}

/// Handle IdentityRotate — rotate keypair, notify friends.
pub(crate) async fn handle_rotate(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let signing_key = match ctx.require_signing_key() {
        Ok(k) => k,
        Err(e) => return e,
    };
    let session = match ctx.require_session(Clone::clone) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let result = match identity::rotate_identity(&transport, &session, &signing_key).await {
        Ok(r) => r,
        Err(e) => return IpcResponse::error(500, format!("identity rotation failed: {e}")),
    };

    // Store new key in OS keyring
    if let Err(e) = crate::state::keystore::store_signing_key(&result.new_signing_key_bytes).await {
        return IpcResponse::error(500, format!("failed to store new signing key: {e}"));
    }

    // Update session
    {
        let mut guard = ctx.session.write();
        if let Some(ref mut s) = *guard {
            s.identity
                .public_key_hex
                .clone_from(&result.new_public_key_hex);
        }
    }
    if let Err(e) = ctx.save_session() {
        return e;
    }

    IpcResponse::ok(&serde_json::json!({
        "status": "rotated",
        "new_public_key": result.new_public_key_hex,
        "friends_notified": result.friends_notified,
    }))
}

/// Handle IdentityDestroy — release the unlock (which ends its DHT records),
/// delete its keys and session, then exit so the daemon restarts with no
/// identity.
pub(crate) async fn handle_destroy(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_write() && state != DaemonState::Locked {
        return state_error(state, "destroy");
    }
    // There must be an identity to destroy.
    if let Err(e) = ctx.require_session(|_| ()) {
        return e;
    }

    if state.can_write() {
        // Ending the unlock ends the session's records: the pool hands them
        // to the node's closer (plan C7.6i, C7.7e).
        if let Err(refused) = release_unlock(ctx).await {
            return refused;
        }
    }

    if let Err(failed) = delete_identity_files(ctx).await {
        return failed;
    }
    if let Err(refused) = transition(ctx, DaemonState::ShuttingDown) {
        return refused;
    }
    ctx.shutdown.request(ExitReason::IdentityDestroyed);
    IpcResponse::ok(&serde_json::json!({ "destroyed": true, "restarting": true }))
}

/// Handle IdentityWipe — factory reset: release the unlock, delete keys and
/// session, then exit; the host deletes the Veilid storage once the
/// transport has stopped, and the daemon restarts empty.
pub(crate) async fn handle_wipe(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_write() && state != DaemonState::Locked {
        return state_error(state, "wipe");
    }
    if state.can_write() {
        if let Err(refused) = release_unlock(ctx).await {
            return refused;
        }
    }
    if let Err(failed) = delete_identity_files(ctx).await {
        return failed;
    }
    if let Err(refused) = transition(ctx, DaemonState::ShuttingDown) {
        return refused;
    }
    ctx.shutdown.request(ExitReason::DataWiped);
    IpcResponse::ok(&serde_json::json!({ "wiped": true, "restarting": true }))
}

/// Operational → Locking → Locked, releasing everything the unlock created.
async fn release_unlock(ctx: &DaemonContext) -> Result<(), IpcResponse> {
    transition(ctx, DaemonState::Locking)?;
    teardown_unlocked(ctx).await;
    transition(ctx, DaemonState::Locked)
}

/// Delete the identity's keyring entries, then its session. A failure
/// answers 500 and leaves the session in place, so the request can be
/// retried rather than half-done.
async fn delete_identity_files(ctx: &DaemonContext) -> Result<(), IpcResponse> {
    crate::state::keystore::delete_all_keys()
        .await
        .map_err(|e| IpcResponse::error(500, format!("keyring cleanup failed: {e}")))?;
    *ctx.session.write() = None;
    match std::fs::remove_file(&ctx.session_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(IpcResponse::error(
            500,
            format!("cannot delete {}: {e}", ctx.session_path.display()),
        )),
    }
}
