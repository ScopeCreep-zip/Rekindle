//! A focus request as sent by `rekindle community focus`, observed by the
//! second daemon through its served member row and the sealed beat path.

use rekindle_codec::member_record::BeatState;
use rekindle_ipc::protocol::{
    ChannelMessageRequest, CommunityRequest, IpcRequest, LifecycleRequest, SettingsRequest,
};
use rekindle_types::dto::community::CommunityMemberView;
use rekindle_types::presence::{
    LastSeenPrecision, PresencePolicyField, PresenceSharingPolicy, SessionLocation, SessionStatus,
    ShareScope,
};

use crate::beat_engine::{reads_state, set_status, AT_ONCE};
use crate::harness::{eventually, Community, COMMUNITY_NAME};

async fn set_field(c: &Community, node: usize, field: PresencePolicyField) {
    c.nodes[node]
        .ok(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunitySetPresencePolicyField {
                community: c.anchor.clone(),
                field,
            },
        )))
        .await;
}

async fn policy(c: &Community, node: usize) -> PresenceSharingPolicy {
    let data = c.nodes[node]
        .ok(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunityGetPresencePolicy {
                community: COMMUNITY_NAME.into(),
            },
        )))
        .await;
    serde_json::from_value(data).expect("presence policy DTO")
}

async fn served_row(c: &Community, reader: usize, member: &str) -> Option<CommunityMemberView> {
    let rows = c.nodes[reader]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityMembers {
                anchor: c.anchor.clone(),
            },
        )))
        .await;
    serde_json::from_value::<Vec<CommunityMemberView>>(rows)
        .expect("member rows")
        .into_iter()
        .find(|row| row.pseudonym_key == member)
}

async fn focus(c: &Community, channel: Option<&str>) {
    c.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::ChannelSetActive {
                community: COMMUNITY_NAME.into(),
                channel: channel.map(str::to_owned),
                kind: "text".into(),
            },
        )))
        .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn channel_focus_is_served_only_with_reciprocated_location_consent() {
    let c = Community::of(2, 9371).await;
    let creator = c.pseudonym(0).await;
    let location = SessionLocation::Text {
        channel_id: c.general_id(0).await,
    };
    assert_eq!(policy(&c, 0).await, PresenceSharingPolicy::default());
    focus(&c, Some("general")).await;
    assert_eq!(
        served_row(&c, 0, &creator).await.expect("own row").location,
        Some(location.clone()),
    );
    set_status(&c, 0, SessionStatus::Busy).await;
    assert!(reads_state(&c, BeatState::Busy, AT_ONCE)
        .await
        .extras
        .is_none());

    set_field(
        &c,
        0,
        PresencePolicyField::ShareLocation(ShareScope::Members),
    )
    .await;
    set_status(&c, 0, SessionStatus::Away).await;
    assert!(reads_state(&c, BeatState::Away, AT_ONCE)
        .await
        .extras
        .is_some());
    assert!(
        eventually(&c.sim, AT_ONCE, || async {
            served_row(&c, 1, &creator)
                .await
                .is_some_and(|row| row.status == "away" && row.location.is_none())
        })
        .await,
        "a peer without reciprocal consent sees the status only"
    );

    focus(&c, None).await;
    set_field(
        &c,
        0,
        PresencePolicyField::LastSeen(LastSeenPrecision::Exact),
    )
    .await;
    assert_eq!(
        policy(&c, 0).await.share_location,
        ShareScope::Members,
        "editing last-seen keeps the existing location consent",
    );
    set_field(
        &c,
        1,
        PresencePolicyField::ShareLocation(ShareScope::Members),
    )
    .await;
    set_status(&c, 0, SessionStatus::Online).await;
    assert!(reads_state(&c, BeatState::Online, AT_ONCE)
        .await
        .extras
        .is_none());
    focus(&c, Some("general")).await;
    assert!(
        eventually(&c.sim, AT_ONCE, || async {
            served_row(&c, 1, &creator)
                .await
                .is_some_and(|row| row.location.as_ref() == Some(&location))
        })
        .await,
        "the opted-in peer is served the focused general channel"
    );

    set_field(&c, 0, PresencePolicyField::ShareLocation(ShareScope::None)).await;
    assert!(
        eventually(&c.sim, AT_ONCE, || async {
            served_row(&c, 1, &creator)
                .await
                .is_some_and(|row| row.status == "online" && row.location.is_none())
        })
        .await,
        "revocation clears a previously received location without changing status"
    );
    assert_eq!(
        policy(&c, 0).await,
        PresenceSharingPolicy {
            last_seen: LastSeenPrecision::Exact,
            ..PresenceSharingPolicy::default()
        },
    );
    assert_eq!(
        served_row(&c, 0, &creator).await.expect("own row").location,
        Some(location),
    );
}
