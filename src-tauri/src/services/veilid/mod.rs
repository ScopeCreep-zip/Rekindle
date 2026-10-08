use std::sync::Arc;

use tauri::AppHandle;
use veilid_core::VeilidUpdate;

use crate::state::AppState;

pub(crate) mod app_call_lane;
mod app_message;
mod control;
mod control_event_records;
mod control_events;
pub(crate) mod control_ingress;
mod control_membership;
mod control_moderation;
mod control_sync;
mod dht_watch;
pub(crate) mod ingress_queue;
mod lifecycle;
mod network;
pub(crate) mod route_publish;

pub(crate) use lifecycle::route_watchdog_loop;
pub use lifecycle::{
    emit_network_status, initialize_node, logout_cleanup, route_status, shutdown_app,
    start_dispatch_loop,
};

pub async fn handle_veilid_update(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    update: VeilidUpdate,
) {
    match update {
        VeilidUpdate::AppMessage(msg) => app_message::handle(app_handle, state, &msg),
        VeilidUpdate::AppCall(call) => network::handle_app_call(app_handle, state, *call).await,
        VeilidUpdate::ValueChange(change) => {
            dht_watch::handle_value_change(app_handle, state, *change).await;
        }
        VeilidUpdate::Attachment(attachment) => {
            network::handle_attachment(app_handle, state, &attachment);
        }
        VeilidUpdate::RouteChange(change) => {
            network::handle_route_change(state, &change);
        }
        VeilidUpdate::Shutdown => {
            tracing::info!("veilid core shutdown event received");
        }
        _ => {}
    }
}
