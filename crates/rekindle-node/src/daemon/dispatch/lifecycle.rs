//! Lifecycle dispatch handlers: Unlock, Lock, Shutdown, and the teardown
//! of an unlock. Status is in `status.rs`.
//!
//! These are always-available or state-gated commands that manage the
//! daemon's own lifecycle rather than performing Veilid operations.

use std::sync::Arc;

use crate::daemon::DaemonState;
use rekindle_ipc::protocol::IpcResponse;

use crate::daemon::shutdown::ExitReason;

use super::{transition, DaemonContext};

/// Handle Unlock — transition from Locked → Resuming → Operational.
pub(crate) async fn handle_unlock(
    ctx: &DaemonContext,
    state: DaemonState,
    _passphrase: &str,
) -> IpcResponse {
    if !state.can_unlock() {
        return IpcResponse::error(409, format!("cannot unlock in state '{}'", state.as_str()));
    }
    if let Err(missing) = ctx.require_session(|_| ()) {
        return missing;
    }
    if let Err(refused) = transition(ctx, DaemonState::Resuming) {
        return refused;
    }

    // Load signing key from OS keyring into memory.
    let signing_key = match crate::state::keystore::load_signing_key().await {
        Ok(handle) => handle,
        Err(e) => {
            if let Err(refused) = transition(ctx, DaemonState::Locked) {
                return refused;
            }
            return IpcResponse::error_with_remediation(
                500,
                format!("failed to load signing key: {e}"),
                "initialize an identity first",
            );
        }
    };
    let signing_bytes = *signing_key.as_bytes();
    *ctx.signing_key.write() = Some(signing_key);

    // Load the profile and friend-list owner keypairs (stored at identity
    // creation as "profile" and "friend_list") into the session. Resume opens
    // both records writable with them, and fails without them. Unlock used
    // to load only the friend list's, so the profile was never writable and
    // the daemon's route blob never reached it (C7.4 finding 2).
    let profile_kp = load_owner_keypair("profile").await;
    let friend_list_kp = load_owner_keypair("friend_list").await;
    {
        let mut guard = ctx.session.write();
        if let Some(ref mut s) = *guard {
            s.identity.profile_keypair_bytes = profile_kp;
            s.identity.friend_list_keypair_bytes = friend_list_kp;
        }
    }

    // Clone transport and session references before the await point.
    // parking_lot::RwLockReadGuard must not be held across await.
    let transport_clone = ctx.transport.read().clone();
    let session_clone = ctx.session.read().clone();
    if let (Some(transport), Some(session)) = (&transport_clone, &session_clone) {
        // The session's record pool, before resume opens anything (C7.3).
        if let Err(e) = transport.start_records() {
            tracing::warn!(error = %e, "record pool not started — staying locked");
            teardown_unlocked(ctx).await;
            if let Err(refused) = transition(ctx, DaemonState::Locked) {
                return refused;
            }
            return IpcResponse::error_with_remediation(
                503,
                format!("record pool: {e}"),
                "unlock again",
            );
        }
        let resumed = transport.resume(session, &signing_bytes).await;
        if let Err(e) = &resumed {
            tracing::warn!(error = %e, "session resume failed — staying locked");
            teardown_unlocked(ctx).await;
            if let Err(refused) = transition(ctx, DaemonState::Locked) {
                return refused;
            }
            return IpcResponse::error_with_remediation(
                503,
                format!("resume failed: {e}"),
                "check the network attachment, then unlock again",
            );
        }
        // The communities' governance and registry leases are held for the
        // session (plan C7.7c); a lease the community already holds goes back.
        for (community_id, leases) in resumed.unwrap_or_default() {
            let merged = ctx
                .community_runtime
                .hold_leases(&community_id, leases, |l| {
                    rekindle_transport::broadcast::dht_writes::key_of(transport, l)
                });
            for lease in merged.surplus {
                rekindle_transport::broadcast::dht_writes::release(transport, lease).await;
            }
        }
    }

    // Every task of this unlock runs in its scope (plan C4).
    let unlock_scope = begin_unlock_scope(ctx);

    // The unlock's one STATUS publisher (plan C7.8c).
    crate::daemon::status::start(ctx, &unlock_scope);

    // The resumed communities' record keepalive, in their scopes, which
    // exist from here on (plan C7.8b).
    if let Some(session) = &session_clone {
        for membership in session.communities.values() {
            crate::daemon::keepalive::start(ctx, &membership.governance_key);
        }
    }

    // Initialize subscription manager (three-tier inbound: watch + gossip + poll)
    if let (Some(transport), Some(session)) = (&transport_clone, &session_clone) {
        let sub_mgr = rekindle_transport::SubscriptionManager::new(
            Arc::clone(transport),
            Arc::clone(&ctx.session),
            unlock_scope.child("subscriptions"),
        );
        sub_mgr.setup_identity(session).await;
        for membership in session.communities.values() {
            sub_mgr.setup_community(membership).await;
        }
        for (peer_key, dm_log_key) in &session.dm_log_keys {
            sub_mgr.setup_dm_peer(peer_key, dm_log_key).await;
        }
        // Typing indicators expire and the dedup cache sheds by TTL only
        // if the maintenance loop sweeps them.
        let started = sub_mgr
            .start_poll_loop(60)
            .and_then(|()| sub_mgr.start_maintenance_loop());
        if let Err(closed) = started {
            sub_mgr.shutdown().await;
            teardown_unlocked(ctx).await;
            if let Err(refused) = transition(ctx, DaemonState::Locked) {
                return refused;
            }
            return IpcResponse::error(500, format!("unlock interrupted: {closed}"));
        }
        tracing::info!(
            watches = sub_mgr.watch_count(),
            communities = session.communities.len(),
            dm_peers = session.dm_log_keys.len(),
            "subscription manager initialized"
        );
        // Notify the IPC server that events are available for delivery.
        // The server's internal delivery task subscribes via the broadcast sender
        // and routes events through the EventRouter to subscribed connections.
        let event_sender = sub_mgr.event_sender().clone();
        *ctx.subscriptions.write() = Some(sub_mgr);
        ctx.event_watch_tx.send_replace(Some(event_sender));

        // Initialize broadcast manager (outbound gossip mesh)
        let bcast_mgr = rekindle_transport::BroadcastManager::new(
            Arc::clone(transport),
            Arc::clone(&ctx.session),
            Arc::clone(&ctx.mek_cache),
        );
        for membership in session.communities.values() {
            bcast_mgr.register_mesh(&membership.governance_key);
        }
        tracing::info!(
            communities = session.communities.len(),
            "broadcast manager initialized"
        );
        *ctx.broadcast_mgr.write() = Some(bcast_mgr);
    }

    // Presence polls last, after the signing key, subscriptions and
    // broadcast manager are all in place — the poll signs its own row
    // and broadcasts through the mesh on its first tick.
    {
        let community_ids: Vec<String> = ctx
            .session
            .read()
            .as_ref()
            .map(|s| s.communities.keys().cloned().collect())
            .unwrap_or_default();
        if !community_ids.is_empty() {
            let count = community_ids.len();
            if ctx
                .presence_start_tx
                .send(
                    crate::daemon::presence_adapter::supervisor::PresenceStartRequest {
                        community_ids,
                    },
                )
                .is_err()
            {
                tracing::warn!("presence supervisor is gone — no roster will be maintained");
            } else {
                tracing::info!(communities = count, "presence polls requested");
            }
        }
    }

    if let Err(refused) = transition(ctx, DaemonState::Operational) {
        return refused;
    }
    IpcResponse::ok(&serde_json::json!({ "state": "operational" }))
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

/// Handle Lock — release everything the unlock created and return to
/// Locked. Locking a locked daemon is refused, as an ssh-agent refuses
/// `SSH_AGENTC_LOCK` while locked (draft-ietf-sshm-ssh-agent §3.7).
pub(crate) async fn handle_lock(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if state == DaemonState::Locked {
        return IpcResponse::error(409, "already locked");
    }
    if let Err(refused) = transition(ctx, DaemonState::Locking) {
        return refused;
    }
    teardown_unlocked(ctx).await;
    if let Err(refused) = transition(ctx, DaemonState::Locked) {
        return refused;
    }
    IpcResponse::ok(&serde_json::json!({ "state": "locked" }))
}

/// How long the unlock scope's tasks get to stop at lock or exit.
const UNLOCK_STOP_DEADLINE: std::time::Duration = rekindle_types::config::SESSION_STOP_DEADLINE;

/// Start the unlock's scope. A panicking task exits the daemon so systemd
/// restarts it from fresh state: the task shared the unlocked identity's
/// state with every other task (`evidence/c4-session-scope-research.md`).
fn begin_unlock_scope(ctx: &DaemonContext) -> Arc<rekindle_lifecycle::SessionScope> {
    let shutdown = Arc::clone(&ctx.shutdown);
    let scope = rekindle_lifecycle::SessionScope::new(
        "unlock",
        Arc::new(move |task| {
            tracing::error!(
                task,
                "an unlock task panicked — exiting for a clean restart"
            );
            shutdown.request(ExitReason::HandlerPanic);
        }),
    );
    *ctx.unlock_scope.write() = Some(Arc::clone(&scope));
    scope
}

/// Release everything an unlock created, last-created first: the unlock
/// scope (presence polls, which sign our presence row every tick, the
/// subscription loops and every other unlock task), the subscription
/// state, the broadcast mesh, the cached channel keys, the event source
/// the bus server delivers from, and finally the signing key, which
/// zeroizes on drop. Shared by Lock, a failed Unlock, Destroy, Wipe and
/// process exit, so no path leaves part of an unlock behind.
pub(crate) async fn teardown_unlocked(ctx: &DaemonContext) {
    // The stop token first, so a task sees stop before any refused call;
    // then draining the record pool releases every unlock task waiting on
    // it, so the scope stops at once. The calls in flight run on (C7.6g).
    let scope = ctx.unlock_scope.write().take();
    if let Some(scope) = &scope {
        scope.token().cancel();
    }
    let transport = ctx.transport.read().clone();
    if let Some(transport) = &transport {
        transport.drain_records();
    }
    if let Some(scope) = scope {
        if let Err(stuck) = scope.shutdown(UNLOCK_STOP_DEADLINE).await {
            tracing::warn!(%stuck, "unlock scope did not stop in time");
        }
    }
    ctx.community_scopes.lock().clear();
    let subscriptions = ctx.subscriptions.write().take();
    if let Some(subscriptions) = subscriptions {
        subscriptions.shutdown().await;
    }
    // Offline, once the unlock's tasks (its status publisher among them)
    // stopped: the teardown's own write, admitted on the drained pool.
    if let Some(transport) = &transport {
        transport.admit_records_teardown();
    }
    crate::daemon::status::publish_offline(ctx).await;
    // The record pool, after everything that still reads or writes records;
    // then this unlock's routes, so none outlives it and nothing is
    // allocated while locked (plan C7.9d).
    if let Some(transport) = transport {
        transport.end_records();
        transport.release_routes();
    }
    // The per-community runtime belongs to this unlock: its lease ids name
    // the pool that just ended (the next pool counts from 0 again), and its
    // keepalive flags its scopes (plan C7.8b).
    ctx.community_runtime.clear();
    drop(ctx.broadcast_mgr.write().take());
    ctx.mek_cache.write().clear();
    ctx.event_watch_tx.send_replace(None);
    *ctx.signing_key.write() = None;
}

/// An owner keypair from the keyring, or `None` (logged) when it is missing
/// or unreadable; resume then reports the record it cannot open writable.
async fn load_owner_keypair(label: &str) -> Option<Vec<u8>> {
    match crate::state::keystore::load_keypair_bytes(label).await {
        Ok(Some(bytes)) => Some(bytes),
        Ok(None) => {
            tracing::warn!(label, "owner keypair missing from the keyring");
            None
        }
        Err(e) => {
            tracing::warn!(label, error = %e, "owner keypair unreadable");
            None
        }
    }
}
