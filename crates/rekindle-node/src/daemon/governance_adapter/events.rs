//! Event emission.
//!
//! The Tauri adapter calls `app.emit` and the SolidJS UI listens. The
//! daemon's equivalent is the `SubscriptionEvent` broadcast that the IPC
//! server subscribes to and fans out through `EventRouter` to connected
//! clients — so a CLI or a third-party TUI sees the same governance
//! changes the desktop UI does. That parity is the point: a capability
//! that only reaches the Tauri frontend is a capability no other
//! frontend can offer.
//!
//! `GovernanceRuntimeEvent` is the crate's host-agnostic vocabulary;
//! `SubscriptionEvent::Governance` is this track's wire form. Mapping
//! between them is the adapter's job.

use rekindle_governance_runtime::event::GovernanceRuntimeEvent;
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};

use super::DaemonGovernanceAdapter;

impl DaemonGovernanceAdapter {
    /// Translate and publish one runtime event.
    ///
    /// Events with no `SubscriptionEvent` counterpart are traced rather
    /// than dropped silently — several are progress/telemetry signals
    /// (`JoinProgress`, `BootstrapResponseBuilt`) that the IPC surface
    /// has no variant for yet. Logging them keeps the gap visible
    /// instead of making it look handled.
    pub(super) fn emit_event_impl(&self, event: &GovernanceRuntimeEvent) {
        let mapped = match event {
            // A member's role set changing and the role table changing
            // are the same signal to a client: re-read roles.
            GovernanceRuntimeEvent::RolesChanged { community_id }
            | GovernanceRuntimeEvent::MemberRolesChanged { community_id, .. } => {
                Some(GovernanceEvent::RolesChanged {
                    community: community_id.clone(),
                })
            }
            GovernanceRuntimeEvent::ChannelsUpdated { community_id } => {
                Some(GovernanceEvent::ChannelsChanged {
                    community: community_id.clone(),
                })
            }
            GovernanceRuntimeEvent::CommunityCreated { community_id, .. }
            | GovernanceRuntimeEvent::CommunityJoined { community_id, .. }
            | GovernanceRuntimeEvent::JoinAccepted { community_id }
            | GovernanceRuntimeEvent::SegmentAdded { community_id, .. } => {
                Some(GovernanceEvent::MetadataChanged {
                    community: community_id.clone(),
                })
            }
            _ => None,
        };

        let Some(governance_event) = mapped else {
            tracing::debug!(
                ?event,
                "governance adapter: no SubscriptionEvent counterpart, not delivered to IPC clients"
            );
            return;
        };

        let guard = self.ctx.subscriptions.read();
        let Some(manager) = guard.as_ref() else {
            // Pre-unlock: nobody is subscribed, so there is nothing to
            // deliver to. Not an error.
            return;
        };
        // `send` fails only when every receiver has dropped, which is
        // the normal state with no clients attached.
        let _ = manager
            .event_sender()
            .send(SubscriptionEvent::Governance(governance_event));
    }
}
