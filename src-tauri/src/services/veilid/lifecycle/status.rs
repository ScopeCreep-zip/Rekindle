use crate::channels::NetworkStatusEvent;
use crate::state::AppState;
use tauri::Emitter;

/// Build and emit a `NetworkStatusEvent` from current `NodeHandle` state.
pub fn emit_network_status(app_handle: &tauri::AppHandle, state: &AppState) {
    let event = {
        let node = state.node.read();
        match node.as_ref() {
            Some(nh) => NetworkStatusEvent {
                attachment_state: nh.attachment_state.clone(),
                is_attached: nh.is_attached,
                public_internet_ready: nh.public_internet_ready,
                has_route: nh.route_blob.is_some(),
            },
            None => NetworkStatusEvent {
                attachment_state: "detached".to_string(),
                is_attached: false,
                public_internet_ready: false,
                has_route: false,
            },
        }
    };
    let _ = app_handle.emit("network-status", &event);
}
