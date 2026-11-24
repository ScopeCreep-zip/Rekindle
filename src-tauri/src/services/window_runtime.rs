//! Phase 23.C — window/network-status helpers lifted from
//! `commands/window.rs`. The window-opener handlers stay as thin
//! delegations to `crate::windows::*`; this file hosts only the
//! `get_network_status` body which reads the Veilid `NodeHandle`
//! AppState fields.

use crate::commands::window::NetworkStatus;
use crate::state::SharedState;

/// The own-records stage as the network status carries it (plan E2.5
/// FC3); `pending` while no Host is unlocked.
fn own_records(
    state: &SharedState,
) -> (
    rekindle_types::subscription_events::OwnRecordsPhase,
    Option<String>,
) {
    state
        .host
        .current()
        .map(|host| host.own_records_stage())
        .unwrap_or_default()
        .phase()
}

pub fn get_network_status_inner(state: &SharedState) -> NetworkStatus {
    let (general, media_route) = crate::services::veilid::route_status(state);
    let has_route = general == rekindle_types::subscription_events::RouteAvailability::Available;
    let own_route_down = crate::services::veilid::route_down(state);
    let can_write = state.lifecycle.state().can_write();
    let bars = state.network_strength.shown();
    let (own_records, own_records_error) = own_records(state);
    let route_paused = crate::services::veilid::route_paused(state);
    let node = state.node.read();
    match node.as_ref() {
        Some(handle) => NetworkStatus {
            attachment_state: handle.attachment_state.clone(),
            is_attached: handle.is_attached,
            public_internet_ready: handle.public_internet_ready,
            has_route,
            media_route,
            can_write,
            bars,
            own_records,
            own_records_error: own_records_error.clone(),
            route_paused,
            own_route_down,
            profile_dht_key: handle.profile_dht_key.clone(),
            friend_list_dht_key: handle.friend_list_dht_key.clone(),
        },
        None => NetworkStatus {
            attachment_state: "detached".to_string(),
            is_attached: false,
            public_internet_ready: false,
            has_route: false,
            media_route,
            can_write,
            bars,
            own_records,
            own_records_error,
            route_paused,
            own_route_down,
            profile_dht_key: None,
            friend_list_dht_key: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use rekindle_types::subscription_events::{
        OwnRecordsPhase, OwnRouteDownSet, OwnRouteOutage, RouteAvailability, RoutePauseCause,
    };

    use crate::commands::window::NetworkStatus;

    fn status(route_paused: Option<RoutePauseCause>) -> serde_json::Value {
        status_down(route_paused, OwnRouteDownSet::default())
    }

    fn status_down(
        route_paused: Option<RoutePauseCause>,
        own_route_down: OwnRouteDownSet,
    ) -> serde_json::Value {
        serde_json::to_value(NetworkStatus {
            attachment_state: "attached_good".into(),
            is_attached: true,
            public_internet_ready: true,
            has_route: false,
            media_route: RouteAvailability::Allocating,
            can_write: true,
            bars: 3,
            own_records: OwnRecordsPhase::Held,
            own_records_error: None,
            route_paused,
            own_route_down,
            profile_dht_key: None,
            friend_list_dht_key: None,
        })
        .unwrap()
    }

    /// Plan RV1g: the network status carries which classes are down and
    /// since when, so a window opened mid-outage shows it at hydrate.
    #[test]
    fn network_status_carries_own_route_down() {
        let none = status(None);
        assert!(none["ownRouteDown"]["general"].is_null());
        assert!(none["ownRouteDown"]["media"].is_null());
        let down = status_down(
            None,
            OwnRouteDownSet {
                general: Some(OwnRouteOutage {
                    since_ms: 1_700_000_000_000,
                    reason: "allocated route failed to test".into(),
                }),
                media: None,
            },
        );
        assert_eq!(
            down["ownRouteDown"]["general"],
            serde_json::json!({"sinceMs": 1_700_000_000_000_u64, "reason": "allocated route failed to test"})
        );
        assert!(down["ownRouteDown"]["media"].is_null());
    }

    /// Plan RV1: the network status answer carries the pause and its cause,
    /// so a window opened while paused shows it at hydrate; `null` when no
    /// class is paused.
    #[test]
    fn the_network_status_answer_carries_the_route_pause() {
        assert_eq!(
            status(Some(RoutePauseCause::Exhausted))["routePaused"],
            "exhausted"
        );
        assert_eq!(
            status(Some(RoutePauseCause::Unreadable))["routePaused"],
            "unreadable"
        );
        assert!(status(None)["routePaused"].is_null());
    }
}
