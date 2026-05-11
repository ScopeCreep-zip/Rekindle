//! Community info events — command results and governance change detection.

use super::CommunityInfoView;
use crate::v2::tui::action::CommandResult;
use rekindle_types::subscription_events::{SubscriptionEvent, GovernanceEvent};

pub fn handle_command_result(view: &mut CommunityInfoView, result: CommandResult) {
    if let CommandResult::CommunityInfoLoaded { detail } = result {
        if detail.governance_key == view.community {
            view.detail = Some(detail);
            view.loading = false;
        }
    }
}

pub fn handle_subscription_event(view: &mut CommunityInfoView, event: &SubscriptionEvent) {
    if let SubscriptionEvent::Governance(
        GovernanceEvent::ChannelsChanged { community }
        | GovernanceEvent::RolesChanged { community }
        | GovernanceEvent::MetadataChanged { community }
    ) = event {
        if *community == view.community {
            view.loading = true;
            view.detail = None;
        }
    }
}
