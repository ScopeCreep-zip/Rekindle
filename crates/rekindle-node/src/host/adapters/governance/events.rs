//! Event emission: `GovernanceRuntimeEvent`, the runtime's host-agnostic
//! vocabulary, mapped onto the `SubscriptionEvent`s the Host publishes to
//! every frontend (the GUI's windows through its bus subscriber, a CLI or
//! TUI over IPC), so each sees the same governance changes.
//!
//! Role and channel tables come from the merged CRDT (the same builders
//! `role list` and `channel list` use), segments from the community store.

use rekindle_governance_runtime::event::{GovernanceRuntimeEvent, JoinStageStatus};
use rekindle_types::subscription_events::{
    GovernanceEvent, MembershipEvent, NotificationEvent, SubscriptionEvent,
};

use rekindle_types::display::CategoryDisplay;

use super::HostGovernance;

impl HostGovernance {
    /// Translate and publish one runtime event; the progress and telemetry
    /// signals with no frontend counterpart are traced.
    pub(super) fn emit_event_impl(&self, event: &GovernanceRuntimeEvent) {
        let Some(mapped) = self.map_event(event) else {
            tracing::debug!(?event, "governance event: no frontend counterpart");
            return;
        };
        self.ctx.publish_event(mapped);
    }

    fn map_event(&self, event: &GovernanceRuntimeEvent) -> Option<SubscriptionEvent> {
        let governance = |e| Some(SubscriptionEvent::Governance(e));
        let membership = |e| Some(SubscriptionEvent::Membership(e));
        match event {
            GovernanceRuntimeEvent::RolesChanged { community_id } => {
                governance(GovernanceEvent::RolesChanged {
                    community: community_id.clone(),
                    roles: crate::daemon::dispatch::governance::role_displays(
                        &self.ctx,
                        community_id,
                    ),
                })
            }
            GovernanceRuntimeEvent::ChannelsUpdated { community_id } => {
                governance(GovernanceEvent::ChannelsChanged {
                    community: community_id.clone(),
                    channels: crate::daemon::dispatch::channel::channel_overviews(
                        &self.ctx,
                        community_id,
                    ),
                    categories: self.category_displays(community_id),
                })
            }
            GovernanceRuntimeEvent::SegmentAdded { community_id, .. } => {
                governance(GovernanceEvent::SegmentsChanged {
                    community: community_id.clone(),
                    segments: self.ctx.communities.segments(community_id),
                })
            }
            GovernanceRuntimeEvent::CommunityCreated { community_id, .. }
            | GovernanceRuntimeEvent::CommunityJoined { community_id, .. } => {
                governance(self.metadata_changed(community_id))
            }
            GovernanceRuntimeEvent::JoinProgress {
                community_id,
                stage_label,
                status,
            } => membership(MembershipEvent::JoinProgress {
                community: community_id.clone(),
                stage: stage_label.clone(),
                status: match status {
                    JoinStageStatus::Started => "started",
                    JoinStageStatus::Done => "done",
                    JoinStageStatus::Failed => "failed",
                    JoinStageStatus::TimedOut => "timedOut",
                }
                .to_string(),
            }),
            GovernanceRuntimeEvent::JoinPendingAlert {
                have_manage_community,
            } => Some(SubscriptionEvent::Notification(
                NotificationEvent::SystemAlert {
                    title: "Joining community…".to_string(),
                    body: if *have_manage_community {
                        "Community is full — expanding capacity. This will take a few seconds."
                    } else {
                        "Community is full — waiting for an admin to expand it. Up to 30 seconds."
                    }
                    .to_string(),
                },
            )),
            // The runtime signals acceptance before the MEK is read or a
            // slot claimed; the gossip path's event carries both.
            GovernanceRuntimeEvent::JoinAccepted { community_id } => {
                membership(MembershipEvent::JoinAccepted {
                    community: community_id.clone(),
                    mek_generation: None,
                    slot_index: None,
                })
            }
            GovernanceRuntimeEvent::MemberRolesChanged {
                community_id,
                pseudonym_hex,
                role_ids,
            } => membership(MembershipEvent::RolesChanged {
                community: community_id.clone(),
                pseudonym: pseudonym_hex.clone(),
                role_ids: role_ids.clone(),
            }),
            GovernanceRuntimeEvent::OnboardingComplete {
                community_id,
                pseudonym_hex,
                role_ids,
            } => membership(MembershipEvent::OnboardingCompleted {
                community: community_id.clone(),
                pseudonym: pseudonym_hex.clone(),
                role_ids: role_ids.clone(),
            }),
            GovernanceRuntimeEvent::BootstrapResponseBuilt { .. }
            | GovernanceRuntimeEvent::GovernanceEntryApplied { .. } => None,
        }
    }

    /// The community's merged metadata (`metadata` is itself optional: a
    /// community with no metadata entry merged has none).
    fn metadata_changed(&self, community_id: &str) -> GovernanceEvent {
        let meta = self
            .ctx
            .communities
            .governance_state(community_id)
            .and_then(|gov| gov.metadata.clone());
        GovernanceEvent::MetadataChanged {
            community: community_id.to_string(),
            name: meta.as_ref().map(|m| m.name.clone()),
            description: meta.as_ref().and_then(|m| m.description.clone()),
            icon_hash: meta.as_ref().and_then(|m| m.icon_hash.clone()),
            banner_hash: meta.and_then(|m| m.banner_hash),
        }
    }

    /// Categories for a community, sorted by position then id.
    ///
    /// There is no `category list` command to share a builder with, so
    /// this is the one place the projection lives; `ChannelsChanged`
    /// carries it beside the channel tree because a channel's
    /// `category_id` is meaningless to a client that has not seen the
    /// categories.
    fn category_displays(&self, community_id: &str) -> Vec<CategoryDisplay> {
        let Some(gov) = self.ctx.communities.governance_state(community_id) else {
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
