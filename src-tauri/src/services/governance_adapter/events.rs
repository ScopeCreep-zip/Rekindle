//! Phase 23.D.4 — `GovernanceRuntimeEvent` mapper extracted from
//! `deps_impl.rs`. The match arm dispatches to `snapshot_roles` /
//! `snapshot_channels_and_categories` (the live AppState read
//! helpers) and emits the resulting `CommunityEvent` /
//! `NotificationEvent` via `event_dispatch::emit_live`.

use rekindle_governance_runtime::{GovernanceRuntimeEvent, JoinStageStatus};

use crate::channels::community_channel::CommunityEvent;
use crate::event_dispatch::emit_live;

use super::{snapshot_channels_and_categories, snapshot_roles, GovernanceAdapter};

pub(super) fn emit_event_impl(adapter: &GovernanceAdapter, event: GovernanceRuntimeEvent) {
    match event {
        GovernanceRuntimeEvent::RolesChanged { community_id } => {
            let roles = snapshot_roles(&adapter.state, &community_id);
            emit_live(
                &adapter.app_handle,
                "community-event",
                &CommunityEvent::RolesChanged {
                    community_id,
                    roles,
                },
            );
        }
        GovernanceRuntimeEvent::ChannelsUpdated { community_id } => {
            let (channels, categories) =
                snapshot_channels_and_categories(&adapter.state, &community_id);
            emit_live(
                &adapter.app_handle,
                "community-event",
                &CommunityEvent::ChannelsUpdated {
                    community_id,
                    channels,
                    categories,
                },
            );
        }
        GovernanceRuntimeEvent::CommunityCreated { community_id, name } => {
            tracing::info!(community = %community_id, %name, "community created (Phase 18 adapter)");
        }
        GovernanceRuntimeEvent::BootstrapResponseBuilt {
            community_id,
            bytes,
            ..
        } => {
            tracing::debug!(community = %community_id, bytes, "bootstrap response built");
        }
        GovernanceRuntimeEvent::JoinProgress {
            community_id,
            stage_label,
            status,
        } => {
            let status = match status {
                JoinStageStatus::Started => "started",
                JoinStageStatus::Done => "done",
                JoinStageStatus::Failed => "failed",
                JoinStageStatus::TimedOut => "timedOut",
            };
            crate::event_dispatch::emit_membership(
                &adapter.app_handle,
                rekindle_types::subscription_events::MembershipEvent::JoinProgress {
                    community: community_id,
                    stage: stage_label,
                    status: status.to_string(),
                },
            );
        }
        GovernanceRuntimeEvent::CommunityJoined { community_id, name } => {
            tracing::info!(community = %community_id, %name, "community joined (Phase 18 adapter)");
        }
        GovernanceRuntimeEvent::SegmentAdded {
            community_id,
            segment_index,
        } => {
            tracing::info!(community = %community_id, segment = segment_index, "segment added");
        }
        GovernanceRuntimeEvent::JoinPendingAlert {
            have_manage_community,
        } => {
            let body = if have_manage_community {
                "Community is full — expanding capacity. This will take a few seconds.".to_string()
            } else {
                "Community is full — waiting for an admin to expand it. Up to 30 seconds."
                    .to_string()
            };
            crate::event_dispatch::emit_notification(
                &adapter.app_handle,
                rekindle_types::subscription_events::NotificationEvent::SystemAlert {
                    title: "Joining community…".to_string(),
                    body,
                },
            );
        }
        GovernanceRuntimeEvent::GovernanceEntryApplied {
            community_id,
            entry: _,
        } => {
            tracing::trace!(community = %community_id, "governance entry applied");
        }
        GovernanceRuntimeEvent::JoinAccepted { community_id } => {
            crate::event_dispatch::emit_membership(
                &adapter.app_handle,
                rekindle_types::subscription_events::MembershipEvent::JoinAccepted {
                    community: community_id,
                    // This path signals acceptance from the governance
                    // runtime, which has not read the MEK or claimed a
                    // slot yet. The gossip path carries both.
                    mek_generation: None,
                    slot_index: None,
                },
            );
        }
        GovernanceRuntimeEvent::MemberRolesChanged {
            community_id,
            pseudonym_hex,
            role_ids,
        } => {
            crate::event_dispatch::emit_membership(
                &adapter.app_handle,
                rekindle_types::subscription_events::MembershipEvent::RolesChanged {
                    community: community_id,
                    pseudonym: pseudonym_hex,
                    role_ids,
                },
            );
        }
        GovernanceRuntimeEvent::OnboardingComplete {
            community_id,
            pseudonym_hex,
            role_ids,
        } => {
            crate::event_dispatch::emit_membership(
                &adapter.app_handle,
                rekindle_types::subscription_events::MembershipEvent::OnboardingCompleted {
                    community: community_id,
                    pseudonym: pseudonym_hex,
                    role_ids,
                },
            );
        }
    }
}
