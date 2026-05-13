//! Dashboard event handling — command results and subscription events.

use super::state::DashboardView;
use crate::v2::tui::action::CommandResult;

#[allow(clippy::cast_precision_loss)]
pub fn handle_command_result(view: &mut DashboardView, result: CommandResult) {
    match result {
        CommandResult::StatusLoaded { snapshot } => {
            view.node_attached = snapshot.is_attached;
            view.node_public_internet = snapshot.public_internet_ready;
            view.node_uptime_secs = snapshot.uptime_secs;
            view.node_peer_count = snapshot.peer_count;
            view.node_route_allocated = snapshot.route_allocated;
            view.active_transfers = snapshot.bulk_transfers_active;
            view.bytes_sent = snapshot.bulk_bytes_sent;
            view.bytes_received = snapshot.bulk_bytes_received;
            if view.peer_history.len() >= 60 { view.peer_history.pop_front(); }
            view.peer_history.push_back(snapshot.peer_count as f64);
            view.loaded = true;
        }
        CommandResult::CommunityListLoaded { communities } => {
            if view.community_history.len() >= 60 { view.community_history.pop_front(); }
            view.community_history.push_back(communities.len() as f64);
            view.communities = communities;
        }
        CommandResult::FriendListLoaded { friends } => {
            view.friends = friends;
        }
        CommandResult::IdentityLoaded { public_key, display_name, .. } => {
            view.identity_public_key = public_key;
            view.identity_display_name = display_name;
        }
        _ => {}
    }
}

pub fn handle_subscription_event(
    view: &mut DashboardView,
    event: &rekindle_types::subscription_events::SubscriptionEvent,
) {
    use rekindle_types::subscription_events::{SubscriptionEvent, NetworkEvent, PresenceEvent, GovernanceEvent};
    match event {
        SubscriptionEvent::Network(NetworkEvent::AttachmentChanged { is_attached, public_internet_ready }) => {
            view.node_attached = *is_attached;
            view.node_public_internet = *public_internet_ready;
        }
        SubscriptionEvent::Presence(PresenceEvent::FriendChanged { peer_key, status, .. }) => {
            if let Some(f) = view.friends.iter_mut().find(|f| f.public_key == *peer_key) {
                f.status.clone_from(status);
            }
        }
        SubscriptionEvent::Governance(GovernanceEvent::ChannelsChanged { .. }) => {
            view.loaded = false; // triggers reload
        }
        _ => {}
    }
}
