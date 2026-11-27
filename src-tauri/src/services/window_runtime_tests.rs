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
        general_route: rekindle_types::subscription_events::RouteAvailability::Idle,
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
