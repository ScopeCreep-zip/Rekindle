//! Community info events — command results and governance change detection.

use super::CommunityInfoView;
use crate::v2::tui::action::CommandResult;
use rekindle_types::subscription_events::{SubscriptionEvent, GovernanceEvent, SocialEvent};

pub fn handle_command_result(view: &mut CommunityInfoView, result: CommandResult) {
    if let CommandResult::CommunityInfoLoaded { detail } = result {
        if detail.governance_key == view.community {
            view.detail = Some(detail);
            view.loading = false;
        }
    }
}

pub fn handle_subscription_event(view: &mut CommunityInfoView, event: &SubscriptionEvent) {
    match event {
        SubscriptionEvent::Governance(
            GovernanceEvent::ChannelsChanged { community }
            | GovernanceEvent::RolesChanged { community }
            | GovernanceEvent::MetadataChanged { community }
        ) if *community == view.community => {
            view.loading = true;
            view.detail = None;
        }
        SubscriptionEvent::Social(SocialEvent::GameServerAdded {
            community, server_id, game_id, label, address, added_by,
        }) if *community == view.community => {
            view.game_servers.push(serde_json::json!({
                "id": server_id, "game_id": game_id,
                "label": label, "address": address, "added_by": added_by,
            }));
        }
        SubscriptionEvent::Social(SocialEvent::GameServerRemoved {
            community, server_id,
        }) if *community == view.community => {
            view.game_servers.retain(|s| s.get("id").and_then(|v| v.as_str()) != Some(server_id));
        }
        _ => {}
    }
}
