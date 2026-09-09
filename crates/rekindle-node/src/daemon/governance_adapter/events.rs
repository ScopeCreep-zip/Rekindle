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

use rekindle_types::display::CategoryDisplay;

use super::DaemonGovernanceAdapter;

impl DaemonGovernanceAdapter<'_> {
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
                // The event carries the new table rather than asking
                // the client to re-read it — the merged state is right
                // here, and `role_displays` is the same builder the
                // `role list` command uses, so both render identically.
                Some(GovernanceEvent::RolesChanged {
                    community: community_id.clone(),
                    roles: crate::daemon::dispatch::governance::role_displays(
                        self.ctx,
                        community_id,
                    ),
                })
            }
            GovernanceRuntimeEvent::ChannelsUpdated { community_id } => {
                Some(GovernanceEvent::ChannelsChanged {
                    community: community_id.clone(),
                    channels: crate::daemon::dispatch::channel::channel_overviews(
                        self.ctx,
                        community_id,
                    ),
                    categories: self.category_displays(community_id),
                })
            }
            GovernanceRuntimeEvent::CommunityCreated { community_id, .. }
            | GovernanceRuntimeEvent::CommunityJoined { community_id, .. }
            | GovernanceRuntimeEvent::JoinAccepted { community_id }
            | GovernanceRuntimeEvent::SegmentAdded { community_id, .. } => {
                // `metadata` is itself optional — a community that has
                // never had a metadata entry merged has none — so this
                // flattens rather than nesting two `Option`s.
                let meta = self
                    .ctx
                    .community_runtime
                    .governance_state(community_id)
                    .and_then(|gov| gov.metadata.clone());
                Some(GovernanceEvent::MetadataChanged {
                    community: community_id.clone(),
                    name: meta.as_ref().map(|m| m.name.clone()),
                    description: meta.as_ref().and_then(|m| m.description.clone()),
                    icon_hash: meta.as_ref().and_then(|m| m.icon_hash.clone()),
                    banner_hash: meta.and_then(|m| m.banner_hash),
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

    /// Categories for a community, sorted by position then id.
    ///
    /// There is no `category list` command to share a builder with, so
    /// this is the one place the projection lives; `ChannelsChanged`
    /// carries it beside the channel tree because a channel's
    /// `category_id` is meaningless to a client that has not seen the
    /// categories.
    fn category_displays(&self, community_id: &str) -> Vec<CategoryDisplay> {
        let Some(gov) = self.ctx.community_runtime.governance_state(community_id) else {
            return Vec::new();
        };
        let mut out: Vec<CategoryDisplay> = gov
            .categories
            .iter()
            .map(|(id, category)| CategoryDisplay {
                id: hex::encode(id.0),
                name: category.name.clone(),
                sort_order: i32::try_from(category.position).unwrap_or(i32::MAX),
            })
            .collect();
        out.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.id.cmp(&b.id)));
        out
    }
}
