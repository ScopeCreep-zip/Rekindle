//! Republish our own routes when they change (plan C7.9b).
//!
//! `OwnRoutes` (rekindle-protocol) owns allocation, death and release; this
//! login-scoped task is the one place that reacts to a new blob. A
//! reallocated route has a new id and a new blob (`route_allocate.rs`), so
//! every surface that carries it is rewritten:
//! - General: the profile route subkey, the mailbox route, each community's
//!   presence row, and a voice re-announce;
//! - Media: a voice re-announce only. The media route is connection data
//!   for a call, so it rides the call's signaling, never the presence row
//!   (plan C7.15).
//!
//! Every change also refreshes the network status the window shows.

use std::sync::Arc;

use rekindle_protocol::own_routes::{RouteClass, RouteState};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;
use crate::state_helpers;

/// Follow both route classes until `stop`. Changes already seen when this
/// starts are the login publish's (it reads the current blob).
pub(crate) async fn run(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    stop: CancellationToken,
) {
    let Some(routes) = state_helpers::own_routes(&state) else {
        return;
    };
    let mut general = routes.state(RouteClass::General);
    let mut media = routes.state(RouteClass::Media);
    general.borrow_and_update();
    media.borrow_and_update();
    super::emit_network_status(&app_handle, &state);
    loop {
        let (class, route) = tokio::select! {
            () = stop.cancelled() => return,
            changed = general.changed() => {
                if changed.is_err() {
                    return;
                }
                (RouteClass::General, general.borrow_and_update().clone())
            }
            changed = media.changed() => {
                if changed.is_err() {
                    return;
                }
                (RouteClass::Media, media.borrow_and_update().clone())
            }
        };
        super::emit_network_status(&app_handle, &state);
        if let RouteState::Available { blob } = route {
            match class {
                RouteClass::General => republish_general(&app_handle, &state, &blob).await,
                RouteClass::Media => republish_media(&state),
            }
        }
    }
}

async fn republish_general(app_handle: &tauri::AppHandle, state: &Arc<AppState>, blob: &[u8]) {
    if let Err(e) = crate::services::message_service::push_profile_update(
        state,
        rekindle_protocol::dht::profile::SUBKEY_ROUTE_BLOB,
        blob.to_vec(),
    )
    .await
    {
        tracing::warn!(error = %e, "failed to republish the route blob to the profile");
    }
    let mailbox_key = state
        .node
        .read()
        .as_ref()
        .and_then(|nh| nh.mailbox_dht_key.clone());
    if let (Some(mailbox_key), Ok(pool)) = (mailbox_key, state_helpers::record_pool(state)) {
        match rekindle_protocol::dht::mailbox::update_mailbox_route(&pool, &mailbox_key, blob).await
        {
            Ok(outcome) if outcome.missed() => {
                tracing::warn!(?outcome, "mailbox route blob not stored at consensus");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "failed to update the mailbox route blob"),
        }
    }
    // Gossip peers re-learn our route from the presence rows and the next
    // PresenceUpdate, which the initial-sync flag re-broadcasts.
    {
        let mut communities = state.communities.write();
        for community in communities.values_mut() {
            if let Some(gossip) = community.gossip.as_mut() {
                gossip.needs_initial_sync = true;
            }
        }
    }
    rewrite_presence_rows(state).await;
    crate::services::voice_adapter::reannounce_voice_route(state);
    // A key request sent while routeless had no way back to us.
    crate::services::login_runtime::recover_behind_meks(app_handle, state);
    tracing::info!(blob_len = blob.len(), "own route republished");
}

fn republish_media(state: &Arc<AppState>) {
    crate::services::voice_adapter::reannounce_voice_route(state);
    tracing::info!("own media route republished");
}

/// Our presence row in every joined community carries the general blob.
async fn rewrite_presence_rows(state: &Arc<AppState>) {
    let community_ids: Vec<String> = state.communities.read().keys().cloned().collect();
    for community_id in &community_ids {
        crate::services::community::presence::registry::write_our_presence(state, community_id)
            .await;
    }
}
