//! Active-channel dispatch, local focus and peer location sharing over sealed beats.

use rekindle_codec::member_record::BeatState;
use rekindle_ipc::protocol::{
    ChannelMessageRequest, CommunityRequest, IpcRequest, LifecycleRequest, SettingsRequest,
};
use rekindle_types::dto::community::CommunityMemberView;
use rekindle_types::presence::{
    LastSeenPrecision, PresencePolicyField, PresenceSharingPolicy, SessionLocation, SessionStatus,
    ShareScope,
};

use rekindle_types::subscription_events::{PresenceEvent, SubscriptionEvent};

use crate::beat_engine::{reads_state, set_status, AT_ONCE};
use crate::harness::{eventually, Community, COMMUNITY_NAME};

fn field_request(community: &str, field: PresencePolicyField) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Settings(
        SettingsRequest::CommunitySetPresencePolicyField {
            community: community.to_owned(),
            field,
        },
    ))
}

async fn set_field(c: &Community, node: usize, field: PresencePolicyField) {
    c.nodes[node].ok(field_request(&c.anchor, field)).await;
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

async fn focus(c: &Community, node: usize, community: &str, channel: Option<&str>) {
    c.nodes[node]
        .ok(IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::ChannelSetActive {
                community: community.into(),
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
    let peer = c.pseudonym(1).await;
    let channel_id = c.general_id(0).await;
    let location = SessionLocation::Text {
        channel_id: channel_id.clone(),
    };
    assert_eq!(policy(&c, 0).await, PresenceSharingPolicy::default());
    focus(&c, 0, &c.anchor, Some(&channel_id)).await;
    assert_eq!(
        served_row(&c, 0, &creator).await.expect("own row").location,
        Some(location.clone()),
    );
    set_status(&c, 0, SessionStatus::Busy).await;
    assert!(reads_state(&c, BeatState::Busy, AT_ONCE)
        .await
        .extras
        .is_none());
    assert!(
        eventually(&c.sim, AT_ONCE, || async {
            served_row(&c, 1, &creator)
                .await
                .is_some_and(|row| row.status == "busy" && row.location.is_none())
        })
        .await,
        "the peer sees status without location when sharing is off by default",
    );

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

    focus(&c, 0, &c.anchor, None).await;
    assert_eq!(
        served_row(&c, 0, &creator).await.expect("own row").location,
        None,
        "clearing focus removes our own location",
    );
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
    let creator_events = c.nodes[0].events().await;
    let peer_events = c.nodes[1].events().await;
    let creator_changed = |event: &SubscriptionEvent| {
        matches!(event, SubscriptionEvent::Presence(PresenceEvent::CommunityMemberChanged {
            community, pseudonym, ..
        }) if community == &c.anchor && pseudonym == &creator)
    };
    let peer_changed = |event: &SubscriptionEvent| {
        matches!(event, SubscriptionEvent::Presence(PresenceEvent::CommunityMemberChanged {
            community, pseudonym, ..
        }) if community == &c.anchor && pseudonym == &peer)
    };

    focus(&c, 0, COMMUNITY_NAME, Some("general")).await;
    focus(&c, 1, &c.anchor, Some(&channel_id)).await;
    assert!(
        eventually(&c.sim, AT_ONCE, || async {
            peer_events.any(creator_changed)
                && creator_events.any(peer_changed)
                && serves_location(&c, 1, &creator, Some(&location)).await
                && serves_location(&c, 0, &peer, Some(&location)).await
        })
        .await,
        "both opted-in members hear a presence update and are served each other's location"
    );

    let updates = peer_events.matching(creator_changed);
    set_field(&c, 0, PresencePolicyField::ShareLocation(ShareScope::None)).await;
    assert!(
        eventually(&c.sim, AT_ONCE, || async {
            peer_events.matching(creator_changed) > updates
                && serves_location(&c, 1, &creator, None).await
                && serves_location(&c, 0, &peer, None).await
        })
        .await,
        "revocation clears the location in both views without either member changing focus"
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

async fn serves_location(
    c: &Community,
    reader: usize,
    member: &str,
    location: Option<&SessionLocation>,
) -> bool {
    served_row(c, reader, member)
        .await
        .is_some_and(|row| row.status == "online" && row.location.as_ref() == location)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_location_edit_must_commit_before_a_beat_can_share_it() {
    let c = Community::of(2, 9373).await;
    let channel = c.general_id(0).await;
    focus(&c, 0, &c.anchor, Some(&channel)).await;
    let db = c.nodes[0].db.current().expect("unlocked database");
    db.call(|conn| {
        conn.execute_batch(
            "CREATE TEMP TRIGGER reject_consent BEFORE UPDATE OF presence_policy ON communities
             BEGIN SELECT RAISE(ABORT, 'consent write rejected'); END;",
        )
    })
    .await
    .expect("install consent write failure");
    let rejected = c.nodes[0]
        .call(field_request(
            &c.anchor,
            PresencePolicyField::ShareLocation(ShareScope::Members),
        ))
        .await
        .expect_err("rejected consent edit");
    assert!(rejected.message.contains("consent write rejected"));

    // Force a real beat after a rejected update. A writer reading a live
    // policy installed before the database write would seal the location.
    set_status(&c, 0, SessionStatus::Busy).await;
    assert!(
        reads_state(&c, BeatState::Busy, AT_ONCE)
            .await
            .extras
            .is_none(),
        "a beat exposed location consent that never committed"
    );
    assert_eq!(policy(&c, 0).await.share_location, ShareScope::None);

    db.call(|conn| conn.execute_batch("DROP TRIGGER reject_consent"))
        .await
        .expect("restore consent writes");
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
    let owner = c.nodes[0].public_key.clone().expect("unlocked owner");
    let rows = db
        .call(move |conn| rekindle_db::repo::communities::load_all(conn, &owner))
        .await
        .expect("read committed policy");
    assert_eq!(
        rows.into_iter()
            .find(|row| row.id == c.anchor)
            .and_then(|row| row.presence_policy)
            .map(|policy| policy.share_location),
        Some(ShareScope::Members)
    );
}
