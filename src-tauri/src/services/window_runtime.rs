//! Phase 23.C — window/network-status helpers lifted from
//! `commands/window.rs`. The window-opener handlers stay as thin
//! delegations to `crate::windows::*`; this file hosts only the
//! `get_network_status` body which reads the Veilid `NodeHandle`
//! AppState fields.

use crate::commands::window::NetworkStatus;
use crate::state::SharedState;

pub fn get_network_status_inner(state: &SharedState) -> NetworkStatus {
    let node = state.node.read();
    match node.as_ref() {
        Some(handle) => NetworkStatus {
            attachment_state: handle.attachment_state.clone(),
            is_attached: handle.is_attached,
            public_internet_ready: handle.public_internet_ready,
            has_route: handle.route_blob.is_some(),
            profile_dht_key: handle.profile_dht_key.clone(),
            friend_list_dht_key: handle.friend_list_dht_key.clone(),
        },
        None => NetworkStatus {
            attachment_state: "detached".to_string(),
            is_attached: false,
            public_internet_ready: false,
            has_route: false,
            profile_dht_key: None,
            friend_list_dht_key: None,
        },
    }
}
