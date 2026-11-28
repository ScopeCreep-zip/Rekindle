//! The quick wipe's requests (plan E7.1): arming and disarming (Protected:
//! the gate checked the identity's passphrase), the panic shortcut's
//! trigger (Routine, refused here while unarmed), and the duress
//! passphrase's wipe at any passphrase prompt.
//!
//! The duress passphrase lives in the unlocked identity's vault header
//! (`rekindle_vault::duress`); the panic shortcut in its
//! `app_settings.panic_shortcut`. Either trigger wipes the whole root,
//! every identity on it (`identity::wipe_now`).

use rekindle_ipc::protocol::IpcResponse;

use crate::daemon::DaemonState;
use crate::host::identity::IdentityError;

use super::identity::{identity_error, wipe_now};
use super::{state_error, DaemonContext};

/// The duress passphrase matched: wipe, and answer nothing further than
/// that the Host is stopping.
pub(crate) async fn on_duress(ctx: &DaemonContext) -> IpcResponse {
    tracing::info!("quick wipe triggered");
    match wipe_now(ctx, ctx.lifecycle.current()).await {
        Ok(()) => IpcResponse::error(503, "the host is stopping"),
        Err(refused) => refused,
    }
}

/// `QuickWipeArm`: the duress passphrase into the vault header (one
/// atomic rewrite), the panic shortcut (or none) into `app_settings`.
pub(crate) async fn handle_arm(
    ctx: &DaemonContext,
    state: DaemonState,
    duress_passphrase: &str,
    shortcut: Option<String>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let (root, pk) = (ctx.root.clone(), owner.clone());
    let duress = zeroize::Zeroizing::new(duress_passphrase.to_string());
    let armed = rekindle_lifecycle::run_blocking(move || {
        // A duress passphrase that opens the vault would wipe every unlock.
        match crate::host::identity::passphrase_matches(&root, &pk, &duress) {
            Ok(true) => return Err(None),
            Ok(false) | Err(IdentityError::Duress) => {}
            Err(e) => return Err(Some(e)),
        }
        rekindle_vault::duress::arm(&root.identity_vault(&pk), &duress)
            .map_err(|e| Some(IdentityError::Vault(e)))
    })
    .await;
    match armed {
        Ok(Ok(())) => {}
        Ok(Err(None)) => {
            return IpcResponse::error(
                400,
                "the duress passphrase must differ from the identity's passphrase",
            )
        }
        Ok(Err(Some(e))) => return identity_error(&e),
        Err(e) => return IpcResponse::error(500, format!("quick wipe not armed: {e}")),
    }
    let stored = shortcut.clone();
    let saved = db
        .call(move |conn| {
            rekindle_db::repo::preferences::set_panic_shortcut(conn, &owner, stored.as_deref())
        })
        .await;
    match saved {
        Ok(()) => {
            changed(ctx, true, shortcut.clone());
            IpcResponse::ok(&serde_json::json!({ "armed": true, "shortcut": shortcut }))
        }
        Err(e) => IpcResponse::error(500, format!("panic shortcut not saved: {e}")),
    }
}

/// `QuickWipeDisarm`: random bytes in the header's duress pair again, the
/// panic shortcut cleared.
pub(crate) async fn handle_disarm(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let vault = ctx.root.identity_vault(&owner);
    let disarmed = rekindle_lifecycle::run_blocking(move || rekindle_vault::duress::disarm(&vault))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r.map_err(|e| e.to_string()));
    if let Err(e) = disarmed {
        return IpcResponse::error(500, format!("quick wipe not disarmed: {e}"));
    }
    let saved = db
        .call(move |conn| rekindle_db::repo::preferences::set_panic_shortcut(conn, &owner, None))
        .await;
    match saved {
        Ok(()) => {
            changed(ctx, false, None);
            IpcResponse::ok(&serde_json::json!({ "armed": false }))
        }
        Err(e) => IpcResponse::error(500, format!("panic shortcut not cleared: {e}")),
    }
}

/// The quick wipe's new state for every frontend (`QuickWipeChanged`): the
/// GUI registers or releases the panic shortcut on it, so a chord is live
/// the moment its arming commits, never only from the next unlock.
fn changed(ctx: &DaemonContext, armed: bool, shortcut: Option<String>) {
    use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};
    ctx.publish_event(SubscriptionEvent::System(SystemEvent::QuickWipeChanged {
        armed,
        shortcut,
    }));
}

/// `QuickWipeTrigger`: the panic shortcut's wipe; 409 while no shortcut
/// is armed (the gate has no precondition hook, plan E7.1).
pub(crate) async fn handle_trigger(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let armed = db
        .call(move |conn| rekindle_db::repo::preferences::panic_shortcut(conn, &owner))
        .await;
    match armed {
        Ok(Some(_)) => {}
        Ok(None) => return IpcResponse::error(409, "the quick wipe's shortcut is not armed"),
        Err(e) => return IpcResponse::error(500, format!("panic shortcut unreadable: {e}")),
    }
    tracing::info!("quick wipe triggered");
    match wipe_now(ctx, state).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "wiped": true })),
        Err(refused) => refused,
    }
}
