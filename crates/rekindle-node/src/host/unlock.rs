//! One unlock pipeline and one teardown, split by `HostMode` per stage
//! (plan E1.1, D14). `Unlock` and `IdentityCreate` run [`run`] on both
//! hosts; `Lock`, `Logout`, the removal of the unlocked identity, a failed
//! unlock, a panicked task and `Host::shutdown` run [`teardown`].
//!
//! Stages, in order:
//! 1. install the identity, register a bus caller's key, begin the unlock
//!    scope (both modes);
//! 2. the session's one record pool, holding the writes the last lock left
//!    unsent (both modes, when the host has a transport);
//! 3. the Host's own stages in the unlock scope: own records, own presence
//!    (the STATUS publisher), idle, game, the audit check (both modes);
//! 4. community resume, keepalive, subscriptions, the broadcast mesh and the
//!    presence polls (daemon only, until E2.1/E3.1/E3.4 port them);
//! 5. the shell's unported stages (`ShellHooks::unlocked`, shell only);
//! 6. Operational.

use std::sync::Arc;

use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::Caller;
use rekindle_lifecycle::SessionScope;
use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

use super::identity::Unlocked;
use super::shell_hooks::{EndReason, UnlockedView};
use crate::daemon::dispatch::{transition, DaemonContext};
use crate::daemon::shutdown::ExitReason;
use crate::daemon::DaemonState;
use crate::host::HostMode;

/// How long the unlock scope's tasks get to stop at a teardown.
const UNLOCK_STOP_DEADLINE: std::time::Duration = rekindle_types::config::SESSION_STOP_DEADLINE;

/// Run the unlock pipeline for `unlocked` (its vault and database are in
/// the host's stores already): Locked → Resuming → Operational. Answers the
/// opened identity (`dto::identity::IdentityView`), or the error a failed
/// stage gives, after the teardown took out everything it started.
pub(crate) async fn run(
    ctx: &DaemonContext,
    caller: &Caller,
    unlocked: Unlocked,
    created: bool,
) -> IpcResponse {
    if let Err(refused) = transition(ctx, DaemonState::Resuming) {
        drop(unlocked);
        super::identity::lock(&ctx.identity_stores()).await;
        return refused;
    }
    // 1. Install, the caller's key, the scope.
    let installed = match super::unlocked::install(ctx, unlocked).await {
        Ok(installed) => installed,
        Err(e) => return settle_locked(ctx, IpcResponse::error(500, e)),
    };
    if let Some(key) = caller.static_key {
        if let Err(e) = crate::daemon::clients::register_unlocking_client(ctx, &key).await {
            tracing::error!(error = %e, "the unlocking client is not registered");
        }
    }
    let scope = begin_scope(ctx);
    // The outbox (plan E2.5, D22) before any stage that sends.
    if let Err(e) = ctx.outbox.start(ctx, &scope, &installed.public_key) {
        return fail(ctx, IpcResponse::error(500, format!("outbox: {e}"))).await;
    }
    // Our presence starts Online with the unlock (its publisher runs only
    // with a network).
    ctx.own_presence.start(&scope);
    let view = rekindle_types::dto::identity::IdentityView {
        public_key: installed.public_key.clone(),
        display_name: installed.display_name.clone(),
    };

    // The own-records stage tells the prekey maintenance (3c) when our
    // prekey record is filled; with no transport it is dropped unsent.
    let (prekeys_ready, prekeys_filled) = tokio::sync::oneshot::channel();

    // 2. The session's one record pool, with what the last lock left unsent.
    let transport = ctx.transport.read().clone();
    if let Some(transport) = &transport {
        match transport.start_records() {
            Ok(pool) => ctx.outbox.restore_unsent(pool).await,
            Err(e) => {
                tracing::warn!(error = %e, "record pool not started — staying locked");
                return fail(
                    ctx,
                    IpcResponse::error_with_remediation(
                        503,
                        format!("record pool: {e}"),
                        "unlock again",
                    ),
                )
                .await;
            }
        }
        // 3a. Own records.
        let stage = super::own_records::OwnRecords {
            transport: Arc::clone(transport),
            db: Arc::clone(&ctx.db),
            vault: ctx.vault.clone(),
            session: Arc::clone(&ctx.session),
            shell: ctx.shell.clone(),
            rng: Arc::clone(&ctx.rng),
            presence: ctx.own_presence.clone(),
            profile_lease: Arc::clone(&ctx.own_profile_lease),
            prekey_lease: Arc::clone(&ctx.own_prekey_lease),
            prekeys_ready: Some(prekeys_ready),
            friends: super::adapters::friends::HostFriends::of(ctx),
            public_key: installed.public_key.clone(),
            display_name: installed.display_name.clone(),
            secret: installed.secret.clone(),
        };
        if let Err(e) = scope.spawn_with_token("own records", |stop| stage.run(stop)) {
            return fail(
                ctx,
                IpcResponse::error(500, format!("unlock interrupted: {e}")),
            )
            .await;
        }
        // 3b. Idle and game.
        let (presence, db, owner) = (
            ctx.own_presence.clone(),
            Arc::clone(&ctx.db),
            installed.public_key.clone(),
        );
        let idle =
            scope.spawn_with_token("idle", |stop| super::idle::run(presence, db, owner, stop));
        let presence = ctx.own_presence.clone();
        let game =
            scope.spawn_with_token("game detection", |stop| super::game::run(presence, stop));
        if let Err(e) = idle.and(game) {
            return fail(
                ctx,
                IpcResponse::error(500, format!("unlock interrupted: {e}")),
            )
            .await;
        }
    }
    // 3c. Friends (plan E2.1): what was decrypted but never applied is
    // applied first (D24, plan E2.3), then the friend and prekey (plan
    // E2.4) maintenance tasks on both hosts (the prekey one waits for the
    // own-records stage to fill our record); on the daemon, the friends' presence watches too (the GUI's
    // friend sync holds them until E2.5).
    rekindle_transport::operations::direct::replay_unprocessed(
        &super::adapters::friends::HostFriends::of(ctx),
    )
    .await;
    if let Err(e) = super::adapters::friends::start_maintenance(ctx, &scope).and_then(|()| {
        super::adapters::friends::start_prekey_maintenance(ctx, &scope, prekeys_filled)
    }) {
        return fail(
            ctx,
            IpcResponse::error(500, format!("unlock interrupted: {e}")),
        )
        .await;
    }
    if ctx.mode == HostMode::Daemon {
        super::adapters::friends::watch_friends(ctx, &scope);
    }
    // 3c'. The DM chains (plan E2.5, D23): with the outbox and the friends
    // up, every accepted friend's chain is diffed from our cursor, so what
    // arrived while we were away is fetched whether or not its hint was.
    let streams = super::adapters::friends::HostFriends::of(ctx);
    scope.spawn_or_drop("dm stream catch-up", async move {
        rekindle_transport::operations::dm::stream::catch_up_all(&streams).await;
    });
    // 3d. The audit check.
    audit(
        ctx,
        &installed.public_key,
        installed.audit.as_ref(),
        created,
    )
    .await;

    // 4. Daemon-only stages.
    if ctx.mode == HostMode::Daemon {
        if let Err(refused) = crate::daemon::dispatch::daemon_stages(ctx, &scope).await {
            return fail(ctx, refused).await;
        }
    }

    // 5. The shell's unported stages.
    if let Some(shell) = ctx.shell.clone() {
        let view = UnlockedView {
            public_key: installed.public_key.clone(),
            display_name: installed.display_name.clone(),
            secret: installed.secret.clone(),
        };
        if let Err(e) = shell.unlocked(&view, &scope).await {
            tracing::error!(error = %e, "the shell's unlock stages failed — locking again");
            return fail(ctx, IpcResponse::error(500, e)).await;
        }
    }

    // 6. Operational.
    if let Err(refused) = transition(ctx, DaemonState::Operational) {
        return refused;
    }
    IpcResponse::ok(&view)
}

/// A stage failed after the install: tear down, return to Locked, answer
/// `error`.
async fn fail(ctx: &DaemonContext, error: IpcResponse) -> IpcResponse {
    teardown(ctx, EndReason::Failed).await;
    settle_locked(ctx, error)
}

fn settle_locked(ctx: &DaemonContext, error: IpcResponse) -> IpcResponse {
    match transition(ctx, DaemonState::Locked) {
        Ok(()) => error,
        Err(refused) => refused,
    }
}

/// Begin the unlock's scope. A panicking task shared the unlocked
/// identity's state with every other task
/// (`evidence/c4-session-scope-research.md`): `rekindled` exits so its
/// supervisor restarts it fresh; the GUI's Host runs a lock teardown, which
/// the shell renders as "Session ended".
fn begin_scope(ctx: &DaemonContext) -> Arc<SessionScope> {
    let on_panic: rekindle_lifecycle::OnPanic = match ctx.mode {
        HostMode::Daemon => {
            let shutdown = Arc::clone(&ctx.shutdown);
            Arc::new(move |task| {
                tracing::error!(
                    task,
                    "an unlock task panicked — exiting for a clean restart"
                );
                shutdown.request(ExitReason::HandlerPanic);
            })
        }
        HostMode::Shell => {
            let panics = ctx.unlock_panics.clone();
            Arc::new(move |task| {
                tracing::error!(task, "an unlock task panicked — ending the session");
                let _ = panics.send(task.to_string());
            })
        }
    };
    let scope = SessionScope::new("unlock", on_panic);
    *ctx.unlock_scope.write() = Some(Arc::clone(&scope));
    scope
}

/// The audit step: the chain verified end to end, the unlock appended, and
/// a truncated or broken chain raised as `AuditChainBroken` with its alert.
async fn audit(
    ctx: &DaemonContext,
    owner: &str,
    restored: Option<&rekindle_audit::Restored>,
    created: bool,
) {
    let Ok(db) = ctx.db.current() else {
        return;
    };
    if let Some(rekindle_audit::Restored::Truncated { anchor_cursor }) = restored {
        super::audit_alert::truncated(ctx, *anchor_cursor);
    }
    super::audit_alert::verify(ctx, &db, owner).await;
    super::audit::append(
        &ctx.audit_chain,
        &db,
        &ctx.vault,
        owner,
        rekindle_audit::AuditKind::VaultUnlocked,
        serde_json::json!({ "first_time": created }),
        ctx.clock.now_ms_i64(),
    );
}

/// Release everything an unlock created, in the plan's order, so no path
/// leaves part of an unlock behind: the shell's leave of calls, the scope
/// (stop token, pool drain, shutdown), the daemon's community and
/// subscription state, the Offline write, the writes still held (saved for
/// the next unlock, with an alert), the record pool, this unlock's routes
/// (released, and wanted again unless the host stops), the identity, and
/// the shell's own cleanup. The caller moves the lifecycle.
pub(crate) async fn teardown(ctx: &DaemonContext, reason: EndReason) {
    let shell = ctx.shell.clone();
    if let Some(shell) = &shell {
        shell.ending().await;
    }
    // The stop token first, so a task sees stop before any refused call;
    // then draining the pool releases every task waiting on it, so the
    // scope stops at once. Calls in flight run on (C7.6g).
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
    ctx.outbox.stop();
    if ctx.mode == HostMode::Daemon {
        crate::daemon::dispatch::daemon_teardown(ctx).await;
    }
    if let Some(transport) = transport {
        // Offline, once the unlock's tasks (its STATUS publisher among
        // them) stopped: the teardown's own write on the drained pool.
        transport.admit_records_teardown();
        ctx.own_presence.publish_offline().await;
        let unsent = transport.take_unsent();
        if !unsent.is_empty() {
            save_unsent(ctx, unsent).await;
        }
        transport.end_records();
        // No route serves two identities (C7.9b): this unlock's go, and the
        // next login gets fresh ones at once.
        transport.release_routes();
        if reason != EndReason::Exit {
            transport.want_routes();
        }
    }
    super::unlocked::release(ctx).await;
    if let Some(shell) = shell {
        shell.ended(reason).await;
    }
}

/// Keep the drained pool's held writes for the next unlock, and say so.
async fn save_unsent(ctx: &DaemonContext, unsent: Vec<rekindle_records::port::UnsentWrite>) {
    let owner = ctx
        .session
        .read()
        .as_ref()
        .map(|s| s.identity.public_key_hex.clone());
    let Some(owner) = owner else {
        tracing::warn!(count = unsent.len(), "unsent DHT writes lost: no identity");
        return;
    };
    let saved = super::adapters::outbox::save_unsent(ctx, &owner, unsent).await;
    if saved > 0 {
        tracing::info!(saved, "unsent DHT writes kept for the next unlock");
        ctx.publish_event(SubscriptionEvent::Notification(
            NotificationEvent::SystemAlert {
                title: "Changes still to publish".into(),
                body: format!(
                    "{saved} profile or contact change(s) did not reach the network before \
                     you logged out. They will be published the next time you log in."
                ),
            },
        ));
    }
}

/// End the session after a task panicked (the shell's scope hook): a lock
/// teardown with `TaskPanicked`, on the Host's worker that owns the
/// context. Runs for the life of the host.
pub(crate) async fn run_panic_teardowns(
    ctx: Arc<DaemonContext>,
    mut panics: tokio::sync::mpsc::UnboundedReceiver<String>,
) {
    while let Some(task) = panics.recv().await {
        if transition(&ctx, DaemonState::Locking).is_err() {
            continue;
        }
        teardown(&ctx, EndReason::TaskPanicked(task)).await;
        let _ = transition(&ctx, DaemonState::Locked);
    }
}
