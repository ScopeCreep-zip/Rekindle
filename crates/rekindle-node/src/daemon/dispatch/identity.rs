//! Identity dispatch handlers: Create, List, Show, Export, Import, Rotate,
//! Delete, Wipe. Every identity's storage is `host::identity`'s (plan D1);
//! these handlers add the daemon's lifecycle and the DHT ceremony.

use base64::Engine as _;
use zeroize::Zeroize;

use rekindle_transport::operations::identity;

use crate::daemon::DaemonState;
use crate::host::identity as host_identity;
use crate::host::identity::IdentityError;
use rekindle_ipc::protocol::IpcResponse;

use crate::daemon::shutdown::ExitReason;

use super::{state_error, store_error, teardown_unlocked, transition, DaemonContext};

fn now_ms() -> u64 {
    rekindle_utils::timestamp_ms()
}

/// An identity failure as the IPC caller sees it: 404 unknown, 401 wrong
/// passphrase, 429 locked out (with the wait), 409 already here, 400 a
/// damaged archive, 500 anything else.
pub(crate) fn identity_error(e: &IdentityError) -> IpcResponse {
    match e {
        IdentityError::NotFound => IpcResponse::error_with_remediation(
            404,
            "no such identity on this device",
            "list them with `rekindle identity list`",
        ),
        IdentityError::WrongPassphrase => IpcResponse::error(401, "wrong passphrase"),
        IdentityError::Locked { retry_after_ms } => IpcResponse::error(
            429,
            format!(
                "too many wrong passphrases; try again in {} s",
                retry_after_ms.div_ceil(1000)
            ),
        ),
        IdentityError::Exists => IpcResponse::error(409, e.to_string()),
        IdentityError::Corrupt(_) => IpcResponse::error(400, e.to_string()),
        _ => IpcResponse::error(500, e.to_string()),
    }
}

/// Handle IdentityCreate: the identity's vault and database
/// (`host::identity::create`), then its DHT records, published with the
/// new identity's own prekey bundle. The daemon stays locked; `Unlock`
/// opens the identity. A failure removes everything the creation wrote.
pub(crate) async fn handle_create(
    ctx: &DaemonContext,
    state: DaemonState,
    display_name: &str,
    passphrase: &str,
) -> IpcResponse {
    if state != DaemonState::Locked {
        return state_error(state, "identity creation (lock first)");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let display_name = display_name.trim().to_owned();
    let stores = ctx.identity_stores();
    let created = match host_identity::create(
        &ctx.root,
        &stores,
        Some(&display_name),
        passphrase,
        rekindle_utils::timestamp_ms_i64(),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return identity_error(&e),
    };
    let public_key = created.public_key.clone();
    let published = publish_new_identity(ctx, &transport, &created).await;
    drop(created);
    host_identity::lock(&stores).await;
    match published {
        Ok(records) => IpcResponse::ok(&serde_json::json!({
            "status": "created",
            "public_key": public_key,
            "display_name": display_name,
            "profile_dht_key": records.profile_dht_key,
            "mailbox_dht_key": records.mailbox_dht_key,
            "friend_list_dht_key": records.friend_list_dht_key,
        })),
        Err(failed) => {
            if let Err(e) = host_identity::remove(&ctx.root, &public_key) {
                tracing::warn!(error = %e, "a failed identity creation left files behind");
            }
            failed
        }
    }
}

/// The creation ceremony's records, written to the new identity's stores:
/// owner keypairs into the vault first, then the record keys.
async fn publish_new_identity(
    ctx: &DaemonContext,
    transport: &rekindle_transport::TransportNode,
    created: &host_identity::Unlocked,
) -> Result<identity::IdentityCreated, IpcResponse> {
    let bundle = created
        .signal
        .current_bundle()
        .map_err(|e| store_error("prekey bundle", e))?
        .to_wire();
    // The ceremony creates our records in a record pool of its own, ended
    // with it: they are re-opened by the unlock that follows (plan C7.4).
    transport
        .start_records()
        .map_err(|e| IpcResponse::error(503, format!("record pool: {e}")))?;
    let ceremony = identity::create_identity(
        transport,
        &created.secret,
        &created.display_name,
        "Hello from Rekindle!",
        &bundle,
    )
    .await;
    transport.end_records();
    let records =
        ceremony.map_err(|e| IpcResponse::error(500, format!("identity creation failed: {e}")))?;

    {
        use rekindle_vault::typed::owner_keys::{persist_owner_keypair, OwnerKey};
        let guard = ctx.vault.lock();
        let vault = guard
            .as_ref()
            .ok_or_else(|| IpcResponse::error(500, "the new identity's vault closed"))?;
        persist_owner_keypair(vault, OwnerKey::Profile, &records.profile_owner_keypair)
            .and_then(|()| {
                persist_owner_keypair(
                    vault,
                    OwnerKey::FriendList,
                    &records.friend_list_owner_keypair,
                )
            })
            .map_err(|e| store_error("owner keypairs", e))?;
    }
    let db = ctx.require_db()?;
    let pk = created.public_key.clone();
    let (profile, friend_list, mailbox, inbox, inbox_keypair) = (
        records.profile_dht_key.clone(),
        records.friend_list_dht_key.clone(),
        records.mailbox_dht_key.clone(),
        records.friend_inbox_key.clone(),
        records.friend_inbox_keypair_hex.clone(),
    );
    db.call(move |conn| {
        use rekindle_db::repo::identity::{
            set_friend_inbox, set_mailbox_key, set_owned_record, OwnedRecord,
        };
        set_owned_record(conn, &pk, OwnedRecord::Profile, &profile)?;
        set_owned_record(conn, &pk, OwnedRecord::FriendList, &friend_list)?;
        set_mailbox_key(conn, &pk, &mailbox)?;
        set_friend_inbox(conn, &pk, &inbox, &inbox_keypair)
    })
    .await
    .map_err(|e| store_error("record keys", e))?;
    Ok(records)
}

/// Handle IdentityList: the registry, in any state (it holds no secret).
pub(crate) fn handle_list(ctx: &DaemonContext) -> IpcResponse {
    match host_identity::list(&ctx.root) {
        Ok(entries) => IpcResponse::ok(&serde_json::json!({ "identities": entries })),
        Err(e) => identity_error(&e),
    }
}

/// Handle IdentityShow — the unlocked identity's public data.
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

/// Handle IdentityExport — the unlocked identity's archive (base64), after
/// its passphrase (a wrong one counts).
pub(crate) fn handle_export(
    ctx: &DaemonContext,
    state: DaemonState,
    passphrase: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let owner = match ctx.require_owner() {
        Ok(o) => o,
        Err(e) => return e,
    };
    match host_identity::export(
        &ctx.root,
        &ctx.identity_stores(),
        &owner,
        passphrase,
        now_ms(),
    ) {
        Ok(archive) => IpcResponse::ok(&serde_json::json!({
            "public_key": owner,
            "archive": base64::engine::general_purpose::STANDARD.encode(archive),
        })),
        Err(e) => identity_error(&e),
    }
}

/// Handle IdentityImport — install an archived identity, locked, in any
/// state (it touches no unlocked store).
pub(crate) async fn handle_import(
    ctx: &DaemonContext,
    archive: &str,
    passphrase: &str,
) -> IpcResponse {
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(archive) else {
        return IpcResponse::error(400, "invalid archive: not base64");
    };
    match host_identity::import(
        &ctx.root,
        &ctx.identity_attempts,
        &bytes,
        passphrase,
        now_ms(),
    )
    .await
    {
        Ok(entry) => IpcResponse::ok(&serde_json::json!({
            "imported": entry.public_key,
            "display_name": entry.display_name,
        })),
        Err(e) => identity_error(&e),
    }
}

/// Handle IdentityRotate — check the passphrase, tell every friend under
/// the old key, lock, then move the identity to its new key
/// (`host::identity::rotate`). The daemon is left locked; the identity
/// unlocks under the new key.
pub(crate) async fn handle_rotate(
    ctx: &DaemonContext,
    state: DaemonState,
    passphrase: &str,
) -> IpcResponse {
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
    let old_key = session.identity.public_key_hex.clone();
    // The passphrase first: no friend hears of a rotation that then fails.
    if let Err(e) = host_identity::verify_passphrase(
        &ctx.root,
        &ctx.identity_attempts,
        &old_key,
        passphrase,
        now_ms(),
    ) {
        return identity_error(&e);
    }

    let mut result = match identity::rotate_identity(&transport, &session, &signing_key).await {
        Ok(r) => r,
        Err(e) => return IpcResponse::error(500, format!("identity rotation failed: {e}")),
    };
    if let Err(refused) = release_unlock(ctx).await {
        result.new_signing_key_bytes.zeroize();
        return refused;
    }
    let rotated = host_identity::rotate(
        &ctx.root,
        &ctx.identity_stores(),
        &old_key,
        &result.new_signing_key_bytes,
        passphrase,
        now_ms(),
    )
    .await;
    result.new_signing_key_bytes.zeroize();
    match rotated {
        Ok(entry) => IpcResponse::ok(&serde_json::json!({
            "status": "rotated",
            "new_public_key": entry.public_key,
            "friends_notified": result.friends_notified,
            "state": "locked",
        })),
        Err(e) => identity_error(&e),
    }
}

/// Handle IdentityDelete — release the unlock if it is this identity's
/// (which ends its DHT records), then remove its files and registry entry.
/// The typed confirmation is the proof until D3.
pub(crate) async fn handle_delete(
    ctx: &DaemonContext,
    state: DaemonState,
    public_key: &str,
) -> IpcResponse {
    if !state.can_write() && state != DaemonState::Locked {
        return state_error(state, "delete");
    }
    let unlocked = ctx
        .session
        .read()
        .as_ref()
        .is_some_and(|s| s.identity.public_key_hex == public_key);
    if unlocked {
        // Ending the unlock ends the session's records: the pool hands them
        // to the node's closer (plan C7.6i, C7.7e).
        if let Err(refused) = release_unlock(ctx).await {
            return refused;
        }
    }
    match host_identity::remove(&ctx.root, public_key) {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "deleted": public_key })),
        Err(e) => identity_error(&e),
    }
}

/// Handle IdentityWipe — factory reset: release the unlock, delete every
/// identity, then exit; the host deletes the Veilid storage once the
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
    // Everything under `state/` but Veilid's storage, which the transport
    // still holds open, and the layout stamp, which stays true.
    let swept = rekindle_db::reset::run(&[rekindle_db::reset::ResetDir {
        path: ctx.root.state.clone(),
        keep: &["veilid", "layout-version"],
    }]);
    if let Err(e) = swept {
        return IpcResponse::error(500, format!("wipe incomplete: {e}"));
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
