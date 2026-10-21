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
    state.communities.write().clear();
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

    // Loaders: preferences (→ E4.1, E5.5), communities and their keys and
    // governance (→ E3.1). Friends are the Host's (plan E2.1).
    services::preferences::load(state, &pool, owner).await?;
    crate::community_loader::load_communities_from_db(&pool, state, owner).await?;
    services::community::hydrate_peer_reliability(state, &pool).await;
    crate::community_loader::restore_community_pseudonyms_and_meks(
        state,
        &state.keystore,
        &view.secret,
    );
    // Last-known governance from the local cache, so communities are usable
    // at once; after the pseudonym restore, which it reads. The DHT rebuild
    // below overwrites it with anything newer.
    if let Err(error) =
        crate::community_loader::restore_governance_from_cache(&pool, state, owner).await
    {
        tracing::warn!(%error, "failed to restore governance from local cache");
    }

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
    let (task_app, task_state) = (app.clone(), Arc::clone(state));
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
        hydrate_communities(&task_app, &task_state, &stop).await;
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

    // The sync loop: its community phase → E3.1, its friend phase → E2.5.
    let (sync_state, sync_pool) = (Arc::clone(state), pool.clone());
    scope.spawn_with_token("sync service", |stop| {
        services::sync_service::start_sync_loop(sync_state, sync_pool, stop)
    })?;

    // The login-time friend sync, once the network is up (→ E2.5).
    let sync_state = Arc::clone(state);
    scope.spawn_with_token("login friend sync", |stop| async move {
        match stop
            .run_until_cancelled(wait_for_network_ready(&sync_state))
            .await
        {
            Some(true) => {
                if let Err(e) = services::sync_service::sync_friends_now(&sync_state).await {
                    tracing::warn!(error = %e, "login-time friend sync failed");
                }
            }
            Some(false) => tracing::warn!("network not ready — the sync loop syncs friends"),
            None => {}
        }
    })?;
    services::friendship::spawn_coordinator(state, scope)?;

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
/// tell the windows to refresh. Its DHT work is record-pool calls, which the
/// lock's drain releases (plan C7.6g).
async fn hydrate_communities(
    app: &tauri::AppHandle,
    state: &SharedState,
    stop: &tokio_util::sync::CancellationToken,
) {
    use rekindle_types::subscription_events::{
        GovernanceEvent, MembershipEvent, SubscriptionEvent,
    };

    crate::services::governance_adapter::open_community_dht_records(state).await;
    crate::services::governance_adapter::hydrate_community_state_from_dht(state).await;
    crate::services::governance_adapter::rebuild_governance_from_dht(state).await;
    crate::services::governance_adapter::republish_active_records(state).await;
    if stop.is_cancelled() {
        return;
    }
    tracing::info!("background DHT hydration complete — governance state rebuilt");
    let community_ids: Vec<String> = state.communities.read().keys().cloned().collect();
    for community in &community_ids {
        crate::event_dispatch::emit_subscription(
            app,
            &SubscriptionEvent::Governance(GovernanceEvent::GovernanceRebuilt {
                community: community.clone(),
            }),
        );
        // The members too, before the presence poll's first tick.
        crate::event_dispatch::emit_membership(
            app,
            MembershipEvent::MembersRefreshed {
                community: community.clone(),
            },
        );
    }
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
/// its readers use, the community keys we are behind on (→ E3.1),
/// and a friend sync now that the friend list is open (→ E2.5).
pub(super) fn own_records(
    state: &SharedState,
    app: Option<&tauri::AppHandle>,
    keys: &OwnRecordKeys,
) {
    if let Some(nh) = state.node.write().as_mut() {
        nh.profile_dht_key.clone_from(&keys.profile);
        nh.friend_list_dht_key.clone_from(&keys.friend_list);
        nh.mailbox_dht_key.clone_from(&keys.mailbox);
    }
    let Some(app) = app else {
        return;
    };
    if state_helpers::our_route_blob(state).is_some() {
        services::community::mek_rotation::recover_behind_meks(app, state);
    }
    let sync_state = Arc::clone(state);
    state_helpers::spawn_in_login(state, "own records friend sync", async move {
        if let Err(e) = services::sync_service::sync_friends_now(&sync_state).await {
            tracing::warn!(error = %e, "friend sync after own records failed");
        }
    });
}
