use rekindle_types::subscription_events::{NetworkEvent, RouteAvailability, SubscriptionEvent};

use std::sync::Arc;

use crate::state::AppState;
use rekindle_lifecycle::LifecycleState;

/// Whether we hold a general route, and what the media route is doing
/// (plan C7.9c): the two route facts a status indicator shows.
pub fn route_status(state: &AppState) -> (bool, RouteAvailability) {
    use rekindle_records::port::RouteClass;
    let Some(routes) = state.own_routes.read().clone() else {
        return (false, RouteAvailability::Idle);
    };
    let media = routes.state(RouteClass::Media).borrow().availability();
    (routes.blob(RouteClass::General).is_some(), media)
}

/// Build and emit a network attachment event from current `NodeHandle` state.
///
/// Also drives the lifecycle's attachment edges (until E5.7 moves
/// attachment handling into the transport handler; Starting → Locked is
/// the Host's, at its start):
///   - lose network mid-session: `Operational/Degraded → Detached`
///   - recover network: `Detached → Operational`
///
/// The FSM rejects same-state and invalid-edge calls internally, so we
/// can fire transitions on every status update without churn or noise.
pub fn emit_network_status(app_handle: &tauri::AppHandle, state: &AppState) {
    let (has_route, media_route) = route_status(state);
    let mut event = {
        let node = state.node.read();
        match node.as_ref() {
            Some(nh) => NetworkEvent::AttachmentChanged {
                attachment_state: nh.attachment_state.clone(),
                is_attached: nh.is_attached,
                public_internet_ready: nh.public_internet_ready,
                has_route,
                media_route,
                can_write: false,
            },
            None => NetworkEvent::AttachmentChanged {
                attachment_state: "detached".to_string(),
                is_attached: false,
                public_internet_ready: false,
                has_route: false,
                media_route: RouteAvailability::Idle,
                can_write: false,
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
        (false, LifecycleState::Operational | LifecycleState::Degraded) => {
            let _ = state.lifecycle.transition(LifecycleState::Detached);
        }
        (true, LifecycleState::Detached) => {
            let _ = state.lifecycle.transition(LifecycleState::Operational);
        }
        _ => {} // No transition needed for this (attach, state) combination.
    }
    // Read after the edges above: whether the Host takes a write now, which
    // a community create waits for (plan E7.13).
    if let NetworkEvent::AttachmentChanged { can_write, .. } = &mut event {
        *can_write = state.lifecycle.state().can_write();
    }

    crate::event_dispatch::emit_subscription(app_handle, &SubscriptionEvent::Network(event));
}

/// Veilid's attachment changed to `attachment`: report it to the
/// indicator's window and emit `NetworkEvent::Strength` when the shown bars
/// change; a held fall is re-read once its hold ends. The Host's rule
/// (`rekindle_node` `daemon/handler.rs`), run here because this shell owns
/// the node until F1 (plan E7.2, live finding L8).
pub fn sample_strength(app_handle: &tauri::AppHandle, state: &Arc<AppState>, attachment: &str) {
    let report = state.network_strength.report(attachment);
    if let Some(bars) = report.publish {
        emit_strength(app_handle, bars);
    }
    if report.hold {
        let app = app_handle.clone();
        let strength = Arc::clone(&state.network_strength);
        crate::state_helpers::login_scope_or_closed(state).spawn_or_drop(
            "network strength hold",
            async move {
                if let Some(bars) = strength.settle_held().await {
                    emit_strength(&app, bars);
                }
            },
        );
    }
}

fn emit_strength(app_handle: &tauri::AppHandle, bars: u8) {
    crate::event_dispatch::emit_subscription(
        app_handle,
        &SubscriptionEvent::Network(NetworkEvent::Strength { bars }),
    );
}
