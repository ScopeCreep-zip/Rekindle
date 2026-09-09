use rekindle_types::subscription_events::{NetworkEvent, SubscriptionEvent};

use crate::state::AppState;
use rekindle_lifecycle::LifecycleState;

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
    let event = {
        let node = state.node.read();
        match node.as_ref() {
            Some(nh) => NetworkEvent::AttachmentChanged {
                attachment_state: nh.attachment_state.clone(),
                is_attached: nh.is_attached,
                public_internet_ready: nh.public_internet_ready,
                has_route: nh.route_blob.is_some(),
            },
            None => NetworkEvent::AttachmentChanged {
                attachment_state: "detached".to_string(),
                is_attached: false,
                public_internet_ready: false,
                has_route: false,
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
