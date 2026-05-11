//! Friend list event handling.

use super::{FriendListView, PendingRequestDisplay, presence_rank};
use crate::v2::tui::action::CommandResult;
use rekindle_types::subscription_events::{SubscriptionEvent, FriendEvent, PresenceEvent};

pub fn handle_command_result(view: &mut FriendListView, result: CommandResult) {
    if let CommandResult::FriendListLoaded { friends } = result {
        view.friends = friends;
        view.friends.sort_by(|a, b| {
            presence_rank(&a.status).cmp(&presence_rank(&b.status)).then(a.display_name.cmp(&b.display_name))
        });
        if !view.friends.is_empty() && view.list_state.selected().is_none() {
            view.list_state.select(Some(0));
        }
        view.loaded = true;
    }
}

pub fn handle_subscription_event(view: &mut FriendListView, event: &SubscriptionEvent) {
    match event {
        SubscriptionEvent::Friend(FriendEvent::RequestReceived { from_key, display_name, message }) => {
            if !view.pending_requests.iter().any(|r| r.public_key == *from_key) {
                view.pending_requests.push(PendingRequestDisplay {
                    public_key: from_key.clone(), display_name: display_name.clone(), message: message.clone(),
                });
            }
        }
        SubscriptionEvent::Friend(FriendEvent::Accepted { peer_key, .. }) => {
            view.pending_requests.retain(|r| r.public_key != *peer_key);
        }
        SubscriptionEvent::Friend(FriendEvent::Removed { peer_key }) => {
            view.friends.retain(|f| f.public_key != *peer_key);
        }
        SubscriptionEvent::Presence(PresenceEvent::FriendChanged { peer_key, status, .. }) => {
            if let Some(f) = view.friends.iter_mut().find(|f| f.public_key == *peer_key) {
                f.status.clone_from(status);
            }
        }
        _ => {}
    }
}
