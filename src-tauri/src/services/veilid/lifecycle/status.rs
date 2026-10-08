use rekindle_types::subscription_events::{NetworkEvent, RouteAvailability, SubscriptionEvent};

use crate::state::AppState;
use rekindle_lifecycle::LifecycleState;

/// Whether we hold a general route, and what the media route is doing
/// (plan C7.9c): the two route facts a status indicator shows.
pub fn route_status(state: &AppState) -> (bool, RouteAvailability) {
    use rekindle_protocol::own_routes::RouteClass;
    let Some(routes) = state.own_routes.read().clone() else {
        return (false, RouteAvailability::Idle);
    };
    let media = routes.state(RouteClass::Media).borrow().availability();
    (routes.blob(RouteClass::General).is_some(), media)
}

/// Build and emit a network attachment event from current `NodeHandle` state.
///
/// Phase 5 — also drives lifecycle transitions reactive to attachment:
///   - first attach: `Starting → Locked` (so login becomes available)
///   - lose network mid-session: `Operational/Degraded → Detached`
///   - recover network: `Detached → Operational`
///
/// The FSM rejects same-state and invalid-edge calls internally, so we
/// can fire transitions on every status update without churn or noise.
pub fn emit_network_status(app_handle: &tauri::AppHandle, state: &AppState) {
    let (has_route, media_route) = route_status(state);
    let event = {
        let node = state.node.read();
        match node.as_ref() {
            Some(nh) => NetworkEvent::AttachmentChanged {
                attachment_state: nh.attachment_state.clone(),
                is_attached: nh.is_attached,
                public_internet_ready: nh.public_internet_ready,
                has_route,
                media_route,
            },
            None => NetworkEvent::AttachmentChanged {
                attachment_state: "detached".to_string(),
                is_attached: false,
                public_internet_ready: false,
                has_route: false,
                media_route: RouteAvailability::Idle,
            },
        }
    };

    // Phase 5 — reactive lifecycle transitions.
    let cur = state.lifecycle.state();
    // By reference: `event` is still emitted below.
    let is_attached = matches!(
        event,
        NetworkEvent::AttachmentChanged {
            is_attached: true,
            ..
        }
    );
    match (is_attached, cur) {
        (true, LifecycleState::Starting) => {
            let _ = state.lifecycle.transition(LifecycleState::Locked);
        }
        (false, LifecycleState::Operational | LifecycleState::Degraded) => {
            let _ = state.lifecycle.transition(LifecycleState::Detached);
        }
        (true, LifecycleState::Detached) => {
            let _ = state.lifecycle.transition(LifecycleState::Operational);
        }
        _ => {} // No transition needed for this (attach, state) combination.
    }

    crate::event_dispatch::emit_subscription(app_handle, &SubscriptionEvent::Network(event));
}
