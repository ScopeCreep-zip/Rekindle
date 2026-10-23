//! The desktop's reaction to our own routes (plan C7.9b, E1.1).
//!
//! `OwnRoutes` (rekindle-protocol) owns allocation, death and release, and
//! the transport's one route republisher writes a new general blob to our
//! profile and mailbox. This task, in the unlock scope, rewrites the
//! surfaces the shell still owns, since a reallocated route has a new id and
//! a new blob (`route_allocate.rs`):
//! - General: each community's presence row and its initial-sync flag
//!   (→ E3.1/E3.4), a voice re-announce (→ E4.1) and the MEK recovery a
//!   routeless key request missed (→ E3.1);
//! - Media: a voice re-announce only. The media route is connection data
//!   for a call, so it rides the call's signaling, never the presence row
//!   (plan C7.15).
//!
//! Every change also refreshes the network status the window shows.

use std::sync::Arc;

use rekindle_records::port::{RouteClass, RouteState};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;
use crate::state_helpers;

/// Follow both route classes until `stop`. Changes already seen when this
/// starts are the unlock's (its own-records stage reads the current blob).
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
                RouteClass::General => republish_general(&state, &blob).await,
                RouteClass::Media => republish_media(&state),
                // Never handed to a peer: nothing to republish.
                RouteClass::Sending => {}
            }
        }
    }
}

async fn republish_general(state: &Arc<AppState>, blob: &[u8]) {
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
    if let Ok(governance) = crate::state_helpers::host_governance(state) {
        governance.recover_behind_meks();
    }
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
