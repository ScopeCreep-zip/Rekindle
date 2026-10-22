//! `ShellHooks::unlocked` and `own_records`: the GUI's unported unlock
//! stages, in the Host's unlock scope. Each names the step that ports it.

use std::sync::Arc;

use rekindle_lifecycle::SessionScope;
use rekindle_node::host::{OwnRecordKeys, UnlockedView};

use crate::services;
use crate::state::{IdentityState, SharedState, UserStatus};
use crate::state_helpers;

/// How long a community stage waits for the network before it runs
/// best-effort, and the friend sync before it gives up.
const NETWORK_READY_WAIT_SECS: u64 = 60;

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
    // warm-cache stage, plan E3.1 C1); this shell's live channel keys come
    // from the vault until E3.5 makes the Host's MEK cache the one.
    services::preferences::load(state, &pool, owner).await?;
    services::community::hydrate_peer_reliability(state, &pool).await;
    crate::state_helpers::restore_meks_from_vault(state);

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

    // Community DHT hydration, then a governance rebuild (→ E3.1).
    let task_state = Arc::clone(state);
    scope.spawn_with_token("community DHT hydration", |stop| async move {
        // On a sparse cold-start routing table the opens fail; past the
        // wait hydration runs anyway, and the keepalive recovers the rest.
        let ready = stop
            .run_until_cancelled(wait_for_network_ready(&task_state))
            .await;
        let Some(ready) = ready else {
            return;
        };
        if !ready {
            tracing::warn!("network not ready — running community DHT hydration best-effort");
        }
        if let Err(e) = hydrate_communities(&task_state, &stop).await {
            tracing::debug!(error = %e, "locked before the community hydration ran");
        }
    })?;

    // History catch-up per community (→ E3.1). Each community's presence
    // poll, keepalive and inspect loop start when hydration hands its
    // records over (`leases::records_ready`).
    let community_ids: Vec<String> = state.communities.read().keys().cloned().collect();
    for community_id in community_ids {
        services::community::join::schedule_history_catchup(Arc::clone(state), community_id);
    }
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

/// Open, hydrate, rebuild and republish every community's records, then
/// each rebuilt community tells the windows to refresh. Its DHT work is record-pool calls, which the
/// lock's drain releases (plan C7.6g).
async fn hydrate_communities(
    state: &SharedState,
    stop: &tokio_util::sync::CancellationToken,
) -> Result<(), rekindle_db::NotLoggedIn> {
    use rekindle_governance_runtime::dht_hydration;
    let governance = crate::state_helpers::host_governance(state)?;
    dht_hydration::open_community_dht_records(&governance).await;
    dht_hydration::hydrate_community_state_from_dht(&governance).await;
    dht_hydration::rebuild_governance_from_dht(&governance).await;
    dht_hydration::republish_active_records(&governance).await;
    if stop.is_cancelled() {
        return Ok(());
    }
    // Each rebuilt community's GovernanceRebuilt and MembersRefreshed
    // come from the rebuild itself (plan E3.1 C1).
    tracing::info!("background DHT hydration complete — governance state rebuilt");
    Ok(())
}

/// Wait for `public_internet_ready` with a live peer, up to
/// [`NETWORK_READY_WAIT_SECS`]: `true` once ready.
async fn wait_for_network_ready(state: &SharedState) -> bool {
    let mut rx = state.network_ready_rx.clone();
    tokio::time::timeout(
        std::time::Duration::from_secs(NETWORK_READY_WAIT_SECS),
        async {
            loop {
                if *rx.borrow_and_update() {
                    return true;
                }
                if rx.changed().await.is_err() {
                    return false;
                }
            }
        },
    )
    .await
    .unwrap_or(false)
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
