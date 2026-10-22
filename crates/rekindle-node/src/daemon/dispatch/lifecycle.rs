//! Lifecycle dispatch handlers: Unlock, Lock and Logout, Shutdown, and the
//! daemon-only stages of an unlock and its teardown. The pipeline itself is
//! `host::unlock` (plan E1.1); Status is in `status.rs`.

use std::sync::Arc;

use crate::daemon::DaemonState;
use crate::host::shell_hooks::EndReason;
use rekindle_ipc::protocol::IpcResponse;

use crate::daemon::shutdown::ExitReason;

use super::{transition, DaemonContext};

/// Handle Unlock — open `identity` (`host::identity::unlock`: its vault and
/// database), then the unlock pipeline
/// (`host::unlock::run`): Locked → Resuming → Operational.
pub(crate) async fn handle_unlock(
    ctx: &DaemonContext,
    state: DaemonState,
    caller: &super::Caller,
    identity: &str,
    passphrase: &str,
) -> IpcResponse {
    // The identity this host already holds: the passphrase proves this
    // client, which gets the unlocking client's clearance (plan D3.1: the
    // TUI attaches to the GUI's session this way).
    if state == DaemonState::Operational && ctx.require_owner().is_ok_and(|owner| owner == identity)
    {
        return attach_client(ctx, caller, identity, passphrase).await;
    }
    if !state.can_unlock() {
        return IpcResponse::error(409, format!("cannot unlock in state '{}'", state.as_str()));
    }
    match crate::host::identity::unlock(
        &ctx.root,
        &ctx.identity_stores(),
        identity,
        passphrase,
        ctx.clock.now_ms(),
    )
    .await
    {
        Ok(unlocked) => crate::host::unlock::run(ctx, caller, unlocked, false).await,
        Err(e) => super::identity::identity_error(&e),
    }
}

/// The daemon's stages of an unlock (`host::unlock` stage 4), until their
/// ports: the subscription manager (E2.1/E3.1), the broadcast mesh (E3.1) and the
/// presence polls (E3.4).
///
/// # Errors
/// The subscription loops could not start.
pub(crate) async fn daemon_stages(
    ctx: &DaemonContext,
    unlock_scope: &Arc<rekindle_lifecycle::SessionScope>,
) -> Result<(), IpcResponse> {
    let transport = ctx.transport.read().clone();
    let session = ctx.session.read().clone();
    let (Some(transport), Some(session)) = (transport, session) else {
        return Ok(());
    };
    // The communities' records, leases and keepalive are the Host's
    // network stage (plan E3.1 C2a, `host::adapters::governance::login_stage`).

    // The subscription manager (three-tier inbound: watch + gossip + poll).
    let sub_mgr = rekindle_transport::SubscriptionManager::new(
        Arc::clone(&transport),
        Arc::clone(&ctx.session),
        unlock_scope.child("subscriptions"),
    );
    for membership in session.communities.values() {
        sub_mgr.setup_community(membership).await;
    }
    // Typing indicators expire and the dedup cache sheds by TTL only if the
    // maintenance loop sweeps them.
    let started = sub_mgr
        .start_poll_loop(60)
        .and_then(|()| sub_mgr.start_maintenance_loop());
    if let Err(closed) = started {
        sub_mgr.shutdown().await;
        return Err(IpcResponse::error(
            500,
            format!("unlock interrupted: {closed}"),
        ));
    }
    tracing::info!(
        watches = sub_mgr.watch_count(),
        communities = session.communities.len(),
        "subscription manager initialized"
    );
    // The bus server's delivery task forwards what the manager publishes.
    let event_sender = sub_mgr.event_sender().clone();
    let meshes = Arc::clone(sub_mgr.meshes());
    *ctx.subscriptions.write() = Some(sub_mgr);
    ctx.event_watch_tx.send_replace(Some(event_sender));

    // The broadcast manager (outbound gossip mesh).
    let bcast_mgr = rekindle_transport::BroadcastManager::new(
        Arc::clone(&transport),
        Arc::clone(&ctx.session),
        Arc::clone(&ctx.mek_cache),
        meshes,
    );
    for membership in session.communities.values() {
        bcast_mgr.register_mesh(&membership.governance_key);
    }
    *ctx.broadcast_mgr.write() = Some(bcast_mgr);

    // Presence polls last, after the signing key, subscriptions and the
    // mesh: the poll signs its own row and broadcasts on its first tick.
    let community_ids: Vec<String> = session.communities.keys().cloned().collect();
    if !community_ids.is_empty() {
        let count = community_ids.len();
        let request =
            crate::daemon::presence_adapter::supervisor::PresenceStartRequest { community_ids };
        if ctx.presence_start_tx.send(request).is_err() {
            tracing::warn!("presence supervisor is gone — no roster will be maintained");
        } else {
            tracing::info!(communities = count, "presence polls requested");
        }
    }
    Ok(())
}

/// The daemon's half of a teardown (`host::unlock::teardown`), after the
/// unlock scope stopped: the community scopes, the subscription state, the
/// per-community runtime (its lease ids name the pool that is ending), the
/// broadcast mesh and the bus's event source.
pub(crate) async fn daemon_teardown(ctx: &DaemonContext) {
    ctx.community_scopes.lock().clear();
    let subscriptions = ctx.subscriptions.write().take();
    if let Some(subscriptions) = subscriptions {
        subscriptions.shutdown().await;
    }
    ctx.communities.clear();
    drop(ctx.broadcast_mgr.write().take());
    ctx.event_watch_tx.send_replace(None);
}

/// Handle Shutdown — initiate graceful daemon shutdown.
///
/// Responds with Ok *before* the process exits so the client gets
/// confirmation: the shutdown signal stops the bus subscriber, which answers
/// every request in flight — this one included — before the bus closes.
pub(crate) fn handle_shutdown(ctx: &DaemonContext) -> IpcResponse {
    if ctx.shutdown.is_requested() {
        return IpcResponse::ok(&serde_json::json!({ "state": "already_shutting_down" }));
    }
    if let Err(refused) = transition(ctx, DaemonState::ShuttingDown) {
        return refused;
    }
    ctx.shutdown.request(ExitReason::Requested);
    IpcResponse::ok(&serde_json::json!({
        "state": "shutting_down",
        "message": "daemon will exit after draining connections",
    }))
}

/// Handle Lock and Logout — release everything the unlock created
/// (`host::unlock::teardown`) and return to Locked. Locking a locked daemon
/// is refused, as an ssh-agent refuses `SSH_AGENTC_LOCK` while locked
/// (draft-ietf-sshm-ssh-agent §3.7).
pub(crate) async fn handle_lock(
    ctx: &DaemonContext,
    state: DaemonState,
    reason: EndReason,
) -> IpcResponse {
    if state == DaemonState::Locked {
        return IpcResponse::error(409, "already locked");
    }
    if let Err(refused) = transition(ctx, DaemonState::Locking) {
        return refused;
    }
    crate::host::unlock::teardown(ctx, reason).await;
    if let Err(refused) = transition(ctx, DaemonState::Locked) {
        return refused;
    }
    IpcResponse::ok(&serde_json::json!({ "state": "locked" }))
}

/// `Unlock` of the identity already unlocked: check the passphrase off the
/// executor for every caller (a wrong one counts against the identity), and
/// give a bus client the unlocking client's clearance.
async fn attach_client(
    ctx: &DaemonContext,
    caller: &super::Caller,
    identity: &str,
    passphrase: &str,
) -> IpcResponse {
    let (root, attempts, pk, pass, now_ms) = (
        ctx.root.clone(),
        std::sync::Arc::clone(&ctx.identity_attempts),
        identity.to_string(),
        zeroize::Zeroizing::new(passphrase.to_string()),
        ctx.clock.now_ms(),
    );
    let verified = rekindle_lifecycle::run_blocking(move || {
        crate::host::identity::verify_passphrase(&root, &attempts, &pk, &pass, now_ms)
    })
    .await;
    match verified {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return super::identity::identity_error(&e),
        Err(e) => return IpcResponse::error(500, format!("passphrase check: {e}")),
    }
    // Only a bus caller has a key to register; the passphrase was checked
    // (and counted) for every caller.
    if let Some(key) = caller.static_key {
        if let Err(e) = crate::daemon::clients::register_unlocking_client(ctx, &key).await {
            return IpcResponse::error(500, format!("client not registered: {e}"));
        }
    }
    let view = ctx.require_session(|session| rekindle_types::dto::identity::IdentityView {
        public_key: session.identity.public_key_hex.clone(),
        display_name: session.identity.display_name.clone(),
    });
    match view {
        Ok(view) => IpcResponse::ok(&view),
        Err(e) => e,
    }
}
