//! Presence events published to IPC subscribers.
//!
//! The desktop emits these to its SolidJS UI; the daemon publishes the
//! same facts on the `SubscriptionEvent` broadcast so a CLI or TUI sees
//! what the desktop sees. That parity is the point of doing this in a
//! shared crate at all.

use rekindle_types::subscription_events::{PresenceEvent, SubscriptionEvent};

use super::DaemonPresenceAdapter;

impl DaemonPresenceAdapter {
    /// Publish one event, if anything is listening.
    fn publish(&self, event: SubscriptionEvent) {
        let guard = self.ctx.subscriptions.read();
        if let Some(manager) = guard.as_ref() {
            // Fails only when every receiver has dropped, which is the
            // normal state with no clients attached.
            let _ = manager.event_sender().send(event);
        }
    }

    /// A member appeared in the registry that we had not seen before.
    ///
    /// Reported as a presence change rather than a membership event:
    /// `MembershipEvent::Joined` means "somebody joined the community",
    /// which is a governance fact. This is "the scan noticed a member",
    /// which happens on every cold start for everyone already present
    /// and would otherwise announce a crowd of spurious joins.
    pub(super) fn emit_member_discovered_impl(
        &self,
        community_id: &str,
        pseudonym_key: &str,
        _display_name: &str,
    ) {
        self.publish(SubscriptionEvent::Presence(
            PresenceEvent::CommunityMemberChanged {
                community: community_id.to_string(),
                pseudonym: pseudonym_key.to_string(),
                status: "online".to_string(),
                game_name: None,
                game_id: None,
            },
        ));
    }

    /// A member the overlay had online has gone quiet.
    pub(super) fn emit_presence_offline_impl(&self, community_id: &str, pseudonym_key: &str) {
        self.publish(SubscriptionEvent::Presence(
            PresenceEvent::CommunityMemberChanged {
                community: community_id.to_string(),
                pseudonym: pseudonym_key.to_string(),
                status: "offline".to_string(),
                game_name: None,
                game_id: None,
            },
        ));
    }
}
