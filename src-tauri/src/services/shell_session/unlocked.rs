//! `ShellHooks::unlocked` and `own_records`: the GUI's unported unlock
//! stages, in the Host's unlock scope. Each names the step that ports it.

use std::sync::Arc;

use rekindle_lifecycle::SessionScope;
use rekindle_node::host::{OwnRecordKeys, UnlockedView};

use crate::services;
use crate::state::{IdentityState, SharedState, UserStatus};
use crate::state_helpers;

/// The unlock's keys and projection into `AppState`, the loaders, and, with
/// a window and a node, the network stages.
///
/// # Errors
/// The identity's database cannot be read, or the scope closed while the
/// stages started; the Host then tears the unlock down.
pub(super) async fn run(
    state: &SharedState,
    app: Option<&tauri::AppHandle>,
    view: &UnlockedView,
    scope: &Arc<SessionScope>,
) -> Result<(), String> {
    // What the shell's unported code reads of the unlock (→ E3.1, E4.1).
    *state.identity_secret.lock() = Some(*view.secret);
    *state.identity.write() = Some(IdentityState {
        public_key: view.public_key.clone(),
        display_name: view.display_name.clone(),
        status: UserStatus::Online,
        status_message: String::new(),
    });
    let pool = state.db.current()?;
    let owner = view.public_key.as_str();
    // Call media keys go out on the Host's outbox (plan E2.5).
    services::voice_signaling_adapter::install_media_key_carrier(state);

    // Loaders: preferences (→ E4.1, E5.5). Friends are the Host's (plan
    // E2.1), and so are the communities and their governance (the Host's
    // warm-cache stage, plan E3.1 C1); this shell's live channel keys are
    // the Host's MEK stage's, through `ShellHooks::on_mek_installed` (plan
    // E3.1 C2c) until E3.5 makes the Host's MEK cache the one.
    services::preferences::load(state, &pool, owner).await?;
    services::community::hydrate_peer_reliability(state, &pool).await;

    // The network stages need a window to report to and a node.
    let staged = match app {
        Some(app) if state.transport.read().is_some() => {
            network_stages(state, app, &pool, scope).map_err(|e| e.to_string())
        }
        _ => Ok(()),
    };
    // The last stage: a friend invite link that arrived before the unlock
    // is reviewed once the unlock is done (plan E2.1).
    let held_invite = state.held_friend_invite.lock().take();
    if let Some(invite) = held_invite {
        let task_state = Arc::clone(state);
        scope.spawn_or_drop("held friend invite", async move {
            crate::deep_links::review_friend_invite(&task_state, invite).await;
        });
    }
    staged
}

/// The unported network stages, each a task of `scope`.
fn network_stages(
    state: &SharedState,
    app: &tauri::AppHandle,
    pool: &rekindle_db::Db,
    scope: &Arc<SessionScope>,
) -> Result<(), rekindle_lifecycle::ScopeClosed> {
    // Voice drop telemetry (→ E4.1).
    services::voice_adapter::spawn_drop_telemetry(state, app, scope)?;

    // Community hydration, rebuild, republish and history catch-up are the
    // Host's network stage (plan E3.1 C2a); each community's presence poll,
    // keepalive and inspect loop start when it hands the records over
    // (`ShellHooks::community_records_ready`).
    services::community::start_peer_reliability_flush(Arc::clone(state), pool.clone());
    services::community::start_delivery_settle(Arc::clone(state), scope)?;
    services::community::start_event_reminders(Arc::clone(state), pool.clone(), scope)?;

    // The desktop's reaction to our routes (→ E3.1/E3.4/E4.1).
    let (route_app, route_state) = (app.clone(), Arc::clone(state));
    scope.spawn_with_token("route reaction", |stop| {
        services::veilid::route_publish::run(route_app, route_state, stop)
    })?;
    // A live 1:1 call's roster follows its peer's route: call signaling
    // carries no routes, so the one route cache is their only source
    // (→ E4.1, which moves the call FSM and this heal into the Host).
    if let Some(node) = state.transport.read().clone() {
        let updates = node.peers().read().updates();
        let heal_state = Arc::clone(state);
        scope.spawn_with_token("call route heal", |stop| {
            services::voice_adapter::heal_call_routes(heal_state, node, updates, stop)
        })?;
    }
    Ok(())
}

/// The Host's own records are up: the keys into the `NodeHandle` projection
/// its readers use and the community keys we are behind on (→ E3.1).
pub(super) fn own_records(state: &SharedState, keys: &OwnRecordKeys) {
    if let Some(nh) = state.node.write().as_mut() {
        nh.profile_dht_key.clone_from(&keys.profile);
        nh.friend_list_dht_key.clone_from(&keys.friend_list);
        nh.mailbox_dht_key.clone_from(&keys.mailbox);
    }
    if state_helpers::our_route_blob(state).is_some() {
        if let Ok(governance) = state_helpers::host_governance(state) {
            governance.recover_behind_meks();
        }
    }
}
