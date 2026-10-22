//! Plan E3.1 F3 (D26): an `alert_moderators` automod match on a received
//! post becomes a `ModReport` in governance, raised as `AutoModAlert` and
//! listed in the moderator queue.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::governance::AutoModAction;
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use crate::harness::{eventually, Community, SimNode};

const LIMIT: Duration = Duration::from_secs(180);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn mod_report_reaches_moderators() {
    let community = Community::of(2, 231).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let owner = &community.nodes[0];
    owner
        .ok(IpcRequest::Community(CommunityRequest::AutomodRuleSet {
            community: community.governance_key.clone(),
            rule_id: None,
            name: "asks for help".into(),
            enabled: true,
            keywords: vec!["help".into()],
            regex_patterns: Vec::new(),
            action: AutoModAction::AlertModerators,
        }))
        .await;
    let alerts = owner.events().await;

    let posted = community
        .post(1, "please help me")
        .await
        .expect("bea posts");
    let id = posted["messageId"]
        .as_str()
        .expect("a message id")
        .to_owned();

    let alerted = eventually(&community.sim, LIMIT, || async {
        SimNode::saw(&alerts, |event| {
            matches!(
                event,
                SubscriptionEvent::System(SystemEvent::AutoModAlert { message_id, .. })
                    if *message_id == id
            )
        })
    })
    .await;
    assert!(alerted, "the moderator is alerted");

    let queue = owner
        .ok(IpcRequest::Community(CommunityRequest::ModReportList {
            community: community.governance_key.clone(),
        }))
        .await;
    let reports = queue.as_array().expect("a report list");
    assert_eq!(reports.len(), 1, "one report for the post: {queue}");
    assert_eq!(reports[0]["messageId"], id.as_str());
}
