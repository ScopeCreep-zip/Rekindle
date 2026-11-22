//! Event emission: `GovernanceRuntimeEvent`, the runtime's host-agnostic
//! vocabulary, mapped onto the `SubscriptionEvent`s the Host publishes to
//! every frontend (the GUI's windows through its bus subscriber, a CLI or
//! TUI over IPC), so each sees the same governance changes.
//!
//! Role and channel tables come from the merged CRDT (the same builders
//! `role list` and `channel list` use), segments from the community store.

use rekindle_codec::governance_op::Uninterpretable;
use rekindle_governance::merge::QuarantineReason;
use rekindle_governance_runtime::event::{GovernanceRuntimeEvent, JoinStageStatus};
use rekindle_types::subscription_events::{
    GovernanceEvent, MembershipEvent, NotificationEvent, OpQuarantineReason, SubscriptionEvent,
};

use rekindle_types::display::CategoryDisplay;

use super::HostGovernance;

/// `GovernanceEvent::GovernanceOpQuarantined` for one op the merge set
/// aside (plan E3.2 S7c.5): hashes and the author as hex, the cause too.
fn op_quarantined(
    community_id: &str,
    op: &rekindle_types::governance::OpHash,
    author: &rekindle_types::id::PseudonymKey,
    reason: &QuarantineReason,
) -> GovernanceEvent {
    GovernanceEvent::GovernanceOpQuarantined {
        community: community_id.to_owned(),
        op: hex::encode(op.0),
        author: hex::encode(author.0),
        reason: match reason {
            QuarantineReason::AfterAuthorRemoval { .. } => OpQuarantineReason::AfterAuthorRemoval,
            QuarantineReason::TwoCheckpointEndorsementsInOneRound { .. } => {
                OpQuarantineReason::TwoCheckpointEndorsementsInOneRound
            }
            QuarantineReason::CheckpointEquivocation { .. } => {
                OpQuarantineReason::CheckpointEquivocation
            }
            QuarantineReason::RecordClaimed { .. } => OpQuarantineReason::RecordClaimed,
            QuarantineReason::AuthorityQuarantined { .. } => {
                OpQuarantineReason::AuthorityQuarantined
            }
        },
        cause: reason.cause().map(|cause| hex::encode(cause.0)),
    }
}

impl HostGovernance {
    /// Translate and publish one runtime event; the progress and telemetry
    /// signals with no frontend counterpart are traced.
    pub(super) fn emit_event_impl(&self, event: &GovernanceRuntimeEvent) {
        // One rebuild, two frontend events: the governance and the members.
        if let GovernanceRuntimeEvent::GovernanceRebuilt { community_id } = event {
            self.prune_head_hints(community_id);
            self.ctx.publish_event(SubscriptionEvent::Governance(
                GovernanceEvent::GovernanceRebuilt {
                    community: community_id.clone(),
                },
            ));
            self.ctx.publish_event(SubscriptionEvent::Membership(
                MembershipEvent::MembersRefreshed {
                    community: community_id.clone(),
                },
            ));
            return;
        }
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
                    subject: None,
                    body: if *have_manage_community {
                        "Community is full — expanding capacity. This will take a few seconds."
                    } else {
                        "Community is full — waiting for an admin to expand it. Up to 30 seconds."
                    }
                    .to_string(),
                },
            )),
            GovernanceRuntimeEvent::MemberRolesChanged {
                community_id,
                pseudonym_hex,
                role_ids,
            } => membership(MembershipEvent::RolesChanged {
                community: community_id.clone(),
                pseudonym: pseudonym_hex.clone(),
                role_ids: role_ids.iter().map(|id| id.to_hex()).collect(),
            }),
            GovernanceRuntimeEvent::OnboardingComplete {
                community_id,
                pseudonym_hex,
                role_ids,
            } => membership(MembershipEvent::OnboardingCompleted {
                community: community_id.clone(),
                pseudonym: pseudonym_hex.clone(),
                role_ids: role_ids.iter().map(|id| id.to_hex()).collect(),
            }),
            GovernanceRuntimeEvent::GovernanceOpQuarantined {
                community_id,
                op,
                author,
                reason,
            } => governance(op_quarantined(community_id, op, author, reason)),
            GovernanceRuntimeEvent::CommunityQuarantined {
                community_id,
                op,
                author,
                what,
            } => governance(GovernanceEvent::GovernanceOpQuarantined {
                community: community_id.clone(),
                op: hex::encode(op.0),
                author: hex::encode(author.0),
                reason: match what {
                    Uninterpretable::Kind(_) => OpQuarantineReason::UnknownOpKind,
                    Uninterpretable::Version => OpQuarantineReason::UnknownOpVersion,
                },
                cause: None,
            }),
            GovernanceRuntimeEvent::CheckpointSnapshotMismatch {
                community_id,
                height,
                digest,
            } => governance(GovernanceEvent::CheckpointSnapshotMismatch {
                community: community_id.clone(),
                height: *height,
                digest: hex::encode(digest),
            }),
            GovernanceRuntimeEvent::CommunitySuccessorOffered {
                community_id,
                new_anchor,
                signer,
                reason,
                creators,
            } => governance(GovernanceEvent::CommunitySuccessorOffered {
                community: community_id.clone(),
                new_anchor: new_anchor.clone(),
                signer: hex::encode(signer.0),
                reason: *reason,
                creators: creators.iter().map(|c| hex::encode(c.0)).collect(),
            }),
            GovernanceRuntimeEvent::SuccessorLinkRequested {
                community_id,
                request,
                recipient,
                new_anchor,
            } => governance(GovernanceEvent::SuccessorLinkRequested {
                community: community_id.clone(),
                request: hex::encode(request.0),
                recipient: hex::encode(recipient.0),
                new_anchor: new_anchor.clone(),
            }),
            GovernanceRuntimeEvent::GovernanceRebuilt { .. }
            | GovernanceRuntimeEvent::BootstrapResponseBuilt { .. }
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
