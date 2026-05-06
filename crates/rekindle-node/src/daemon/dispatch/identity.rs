//! Identity dispatch handlers: Create, Show, Export, Rotate, Destroy, Wipe.

use zeroize::Zeroize;

use rekindle_transport::operations::identity;
use rekindle_transport::session::{Session, SessionIdentity};

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;
use crate::validation;

use super::{DaemonContext, state_error};

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
        return IpcResponse::error(
            409,
            "identity already exists — destroy first: rekindle identity destroy",
        );
    }

    let display_name = match validation::validate_display_name(display_name) {
        Ok(n) => n,
        Err(e) => return e,
    };

    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };

    // In-memory Signal stores for the ceremony
    let prekey_store = Box::new(rekindle_transport::crypto::signal_store::MemoryPreKeyStore::new());
    let session_store = Box::new(rekindle_transport::crypto::signal_store::MemorySessionStore::new());

    let mut created = match identity::create_identity(
        &transport,
        &display_name,
        "Hello from Rekindle!",
        prekey_store,
        session_store,
    ).await {
        Ok(c) => c,
        Err(e) => return IpcResponse::error(500, format!("identity creation failed: {e}")),
    };

    // Store signing key in OS keyring
    if let Err(e) = crate::state::keystore::store_signing_key(&created.signing_key_bytes).await {
        return IpcResponse::error(500, format!("failed to store signing key: {e}"));
    }

    // Store keypair bytes
    if let Err(e) = crate::state::keystore::store_keypair_bytes("profile", &created.profile_keypair_bytes).await {
        return IpcResponse::error(500, format!("failed to store profile keypair: {e}"));
    }
    if let Err(e) = crate::state::keystore::store_keypair_bytes("friend_list", &created.friend_list_keypair_bytes).await {
        return IpcResponse::error(500, format!("failed to store friend list keypair: {e}"));
    }

    // Store prekey material
    let (spk_id, ref spk_bytes) = created.prekey_material.signed_prekey;
    if let Err(e) = crate::state::keystore::store_keypair_bytes(
        &format!("signed-prekey-{spk_id}"),
        spk_bytes,
    ).await {
        return IpcResponse::error(500, format!("failed to store signed prekey: {e}"));
    }
    for (otpk_id, ref otpk_bytes) in &created.prekey_material.one_time_prekeys {
        if let Err(e) = crate::state::keystore::store_keypair_bytes(
            &format!("one-time-prekey-{otpk_id}"),
            otpk_bytes,
        ).await {
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
    }).unwrap_or_else(|e| e)
}

/// Handle IdentityExport — return identity metadata for client-side file write.
pub(crate) fn handle_export(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_query() { return state_error(state, "query"); }
    ctx.require_session(|session| {
        IpcResponse::ok(&serde_json::json!({
            "public_key": session.identity.public_key_hex,
            "display_name": session.identity.display_name,
            "profile_dht_key": session.identity.profile_dht_key,
            "mailbox_dht_key": session.identity.mailbox_dht_key,
            "friend_list_dht_key": session.identity.friend_list_dht_key,
        }))
    }).unwrap_or_else(|e| e)
}

/// Handle IdentityRotate — rotate keypair, notify friends.
pub(crate) async fn handle_rotate(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let signing_key = match ctx.require_signing_key() { Ok(k) => k, Err(e) => return e };
    let session = match ctx.require_session(Clone::clone) { Ok(s) => s, Err(e) => return e };

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
            s.identity.public_key_hex.clone_from(&result.new_public_key_hex);
        }
    }
    if let Err(e) = ctx.save_session() { return e; }

    IpcResponse::ok(&serde_json::json!({
        "status": "rotated",
        "new_public_key": result.new_public_key_hex,
        "friends_notified": result.friends_notified,
    }))
}

/// Handle IdentityDestroy — close DHT records, delete keyring, delete session.
pub(crate) async fn handle_destroy(
    ctx: &DaemonContext,
    state: DaemonState,
    confirmation: &str,
) -> IpcResponse {
    if confirmation != "DESTROY MY IDENTITY" {
        return IpcResponse::error(400, "confirmation must be exactly 'DESTROY MY IDENTITY'");
    }
    if !state.can_write() { return state_error(state, "write"); }

    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let session = match ctx.require_session(Clone::clone) { Ok(s) => s, Err(e) => return e };

    // Close all DHT records via transport
    if let Err(e) = identity::destroy_identity(&transport, &session).await {
        tracing::warn!(error = %e, "DHT record cleanup failed (continuing with local destroy)");
    }

    // Zeroize signing key
    *ctx.signing_key.write() = None;

    // Delete keyring entries
    if let Err(e) = crate::state::keystore::delete_all_keys().await {
        tracing::warn!(error = %e, "keyring cleanup failed");
    }

    // Clear session from memory and delete file
    *ctx.session.write() = None;
    if ctx.session_path.exists() {
        let _ = std::fs::remove_file(&ctx.session_path);
    }

    ctx.lifecycle.transition(DaemonState::Locked);
    IpcResponse::ok(&serde_json::json!({ "destroyed": true }))
}

/// Handle IdentityWipe — factory reset everything.
pub(crate) async fn handle_wipe(
    ctx: &DaemonContext,
    _state: DaemonState,
    confirmation: &str,
) -> IpcResponse {
    if confirmation != "WIPE ALL DATA" {
        return IpcResponse::error(400, "confirmation must be exactly 'WIPE ALL DATA'");
    }

    // Zeroize signing key
    *ctx.signing_key.write() = None;

    // Delete keyring
    let _ = crate::state::keystore::delete_all_keys().await;

    // Clear session
    *ctx.session.write() = None;
    if ctx.session_path.exists() {
        let _ = std::fs::remove_file(&ctx.session_path);
    }

    // Delete Veilid storage
    let state_paths = match crate::state::StatePaths::resolve() {
        Ok(p) => p,
        Err(e) => return IpcResponse::error(500, format!("cannot resolve paths: {e}")),
    };
    if state_paths.veilid_dir.exists() {
        let _ = std::fs::remove_dir_all(&state_paths.veilid_dir);
    }

    ctx.lifecycle.transition(DaemonState::Locked);
    IpcResponse::ok(&serde_json::json!({ "wiped": true }))
}
