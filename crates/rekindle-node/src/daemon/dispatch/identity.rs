//! Identity dispatch handlers: Create, List, Show, Export, Import, Rotate,
//! Delete, Wipe. Every identity's storage is `host::identity`'s (plan D1);
//! these handlers add the lifecycle (`host::unlock`).

use base64::Engine as _;
use zeroize::Zeroize;

use rekindle_transport::operations::identity;

use crate::daemon::DaemonState;
use crate::host::identity as host_identity;
use crate::host::identity::IdentityError;
use crate::host::shell_hooks::EndReason;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::Caller;
use rekindle_types::dto::identity::{IdentityShown, IdentitySummary};

use crate::daemon::shutdown::ExitReason;

use super::{state_error, transition, DaemonContext};

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
        IdentityError::WrongPassphrase => IpcResponse::error(401, "Wrong passphrase"),
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
/// (`host::identity::create`), then the one unlock pipeline
/// (`host::unlock::run`), whose own-records stage creates its DHT records
/// once the network has a route. No transport is needed: an identity made
/// offline publishes when the network comes. A failure before the unlock
/// removes everything the creation wrote.
pub(crate) async fn handle_create(
    ctx: &DaemonContext,
    state: DaemonState,
    caller: &Caller,
    display_name: Option<&str>,
    passphrase: &str,
) -> IpcResponse {
    if state != DaemonState::Locked {
        return state_error(state, "identity creation (lock first)");
    }
    let display_name = display_name.map(str::trim);
    let created = match host_identity::create(
        &ctx.root,
        &ctx.identity_stores(),
        display_name,
        passphrase,
        ctx.clock.now_ms_i64(),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return identity_error(&e),
    };
    crate::host::unlock::run(ctx, caller, created, true).await
}

/// Handle IdentityList: the registry, oldest first, in any state (it holds
/// no secret).
pub(crate) fn handle_list(ctx: &DaemonContext) -> IpcResponse {
    match host_identity::list(&ctx.root) {
        Ok(entries) => IpcResponse::ok(
            &entries
                .into_iter()
                .map(|entry| IdentitySummary {
                    public_key: entry.public_key,
                    display_name: entry.display_name,
                    created_at: entry.created_at,
                })
                .collect::<Vec<_>>(),
        ),
        Err(e) => identity_error(&e),
    }
}

/// Handle IdentityShow — the unlocked identity's public data, `None` while
/// locked.
pub(crate) fn handle_show(ctx: &DaemonContext) -> IpcResponse {
    let shown = ctx.session.read().as_ref().map(|session| IdentityShown {
        public_key: session.identity.public_key_hex.clone(),
        display_name: session.identity.display_name.clone(),
        profile_dht_key: session.identity.profile_dht_key.clone(),
        mailbox_dht_key: session.identity.mailbox_dht_key.clone(),
        friend_list_dht_key: session.identity.friend_list_dht_key.clone(),
        communities: session.communities.len(),
    });
    IpcResponse::ok(&shown)
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
        ctx.clock.now_ms(),
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
        ctx.clock.now_ms(),
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
    if let Err(e) = ctx.require_transport() {
        return e;
    }
    let old_key = match ctx.require_owner() {
        Ok(k) => k,
        Err(e) => return e,
    };
    // The passphrase first: no friend hears of a rotation that then fails.
    if let Err(e) = host_identity::verify_passphrase(
        &ctx.root,
        &ctx.identity_attempts,
        &old_key,
        passphrase,
        ctx.clock.now_ms(),
    ) {
        return identity_error(&e);
    }

    let mut result = match identity::rotate_identity(
        &crate::host::adapters::friends::HostFriends::of(ctx),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return IpcResponse::error(500, format!("identity rotation failed: {e}")),
    };
    if let Err(refused) = release_unlock(ctx, EndReason::Logout).await {
        result.new_signing_key_bytes.zeroize();
        return refused;
    }
    let rotated = host_identity::rotate(
        &ctx.root,
        &ctx.identity_stores(),
        &old_key,
        &result.new_signing_key_bytes,
        passphrase,
        ctx.clock.now_ms(),
        result.prekeys.take(),
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
        if let Err(refused) = release_unlock(ctx, EndReason::Deleted).await {
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
        if let Err(refused) = release_unlock(ctx, EndReason::Deleted).await {
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
async fn release_unlock(ctx: &DaemonContext, reason: EndReason) -> Result<(), IpcResponse> {
    transition(ctx, DaemonState::Locking)?;
    crate::host::unlock::teardown(ctx, reason).await;
    transition(ctx, DaemonState::Locked)
}
