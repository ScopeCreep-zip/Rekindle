//! Live A/B findings F7b-1 and F7b-2 (2026-11-17), Backend Owns Policy:
//! every frontend gets each membership verdict once, from the merged
//! governance, with its full payload. A kick reaches the kicked member and
//! the others as one `Kicked` naming the remover and carrying the reason
//! (the gossip echo, which carries neither, used to arrive first); a leave
//! reaches the leaver's own frontends and every other member as one
//! `Left`, whether or not the departure gossip reached them. A join goes
//! out at once with no name, and the joiner's verified card names it once
//! read (plan E3.2 F9-2). A card names a member only with the name set for
//! that community, never the identity's; unset, every surface shows the
//! short pseudonym fingerprint, and the verified card cached on the roster
//! row survives a restart (plan E3.4 S6).

use rekindle_types::dto::community_profile::{CommunityProfilePatch, ProfileField};
use std::time::Duration;

use rekindle_ipc::protocol::{
    CommunityRequest, IpcRequest, LifecycleRequest, MemberRequest, SettingsRequest,
};
use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, Events, PASSPHRASE};
use crate::moderation::listed;

/// Why the creator kicks the member.
const REASON: &str = "f7b reason";

/// How many `Kicked` events about `target` `events` holds, and how many of
/// them name `by` and carry [`REASON`].
fn kicked(events: &Events, target: &str, by: &str) -> (usize, usize) {
    let about = |e: &SubscriptionEvent, full: bool| {
        matches!(e, SubscriptionEvent::Membership(MembershipEvent::Kicked {
            target_pseudonym, removed_by, reason, ..
        }) if target_pseudonym == target
            && (!full || (removed_by.as_deref() == Some(by) && reason.as_deref() == Some(REASON))))
    };
    (
        events.matching(|e| about(e, false)),
        events.matching(|e| about(e, true)),
    )
}

/// How many `Left` events about `leaver` `events` holds.
fn left(events: &Events, leaver: &str) -> usize {
    events.matching(|e| {
        matches!(e, SubscriptionEvent::Membership(MembershipEvent::Left { pseudonym, .. })
            if pseudonym == leaver)
    })
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_kick_is_one_kicked_with_its_reason_on_every_member() {
    let community = Community::of(3, 717).await;
    let ann = community.pseudonym(0).await;
    let bea = community.pseudonym(1).await;
    let bea_events = community.nodes[1].events().await;
    let cid_events = community.nodes[2].events().await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.anchor.clone(),
                target_pseudonym: bea.clone(),
                reason: Some(REASON.into()),
            },
        )))
        .await;
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            community.info(1).await["removed"]["by"].as_str() == Some(ann.as_str())
                && listed(&community, 2, &bea).await.is_none()
        })
        .await,
        "the kick merges on the kicked member and on the third"
    );
    // Long past the gossip echo and a rebuild round.
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.sim.quiesce().await;
    for (node, events) in [(1, &bea_events), (2, &cid_events)] {
        assert_eq!(
            kicked(events, &bea, &ann),
            (1, 1),
            "node {node} gets exactly one Kicked, naming the remover and the reason"
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_leave_is_one_left_on_the_leaver_and_every_member() {
    let community = Community::of(3, 719).await;
    let bea = community.pseudonym(1).await;
    let bea_events = community.nodes[1].events().await;
    let cid_events = community.nodes[2].events().await;
    community.nodes[1]
        .call_held(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityLeave {
                anchor: community.anchor.clone(),
            },
        )))
        .await
        .expect("the leave commits");
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            listed(&community, 2, &bea).await.is_none()
        })
        .await,
        "the leave merges on the third member"
    );
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.sim.quiesce().await;
    assert_eq!(
        left(&bea_events, &bea),
        1,
        "the leaver's own frontends see its leave once"
    );
    assert_eq!(
        left(&cid_events, &bea),
        1,
        "another member sees the leave once"
    );
    let forgotten = |events: &Events| {
        events.matching(|e| {
            matches!(e, SubscriptionEvent::Membership(MembershipEvent::Forgotten { community: c })
                if *c == community.anchor)
        })
    };
    assert_eq!(
        forgotten(&bea_events),
        1,
        "the leaver's every frontend is told once that the community is gone"
    );
    assert_eq!(forgotten(&cid_events), 0, "another member forgets nothing");
}

/// Plan E3.2 F9-2 (A/B round 9 saw `displayName: ""`): another member gets
/// one `Joined` with no name, never "", and then one `MemberCardChanged`
/// with the name on the joiner's verified card.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_join_is_named_by_the_joiners_card_once_read() {
    let mut community = Community::create(2, 721).await;
    let ann_events = community.nodes[0].events().await;
    community.join(1).await;
    let bea = community.pseudonym(1).await;
    let card = |e: &SubscriptionEvent| {
        matches!(e, SubscriptionEvent::Membership(MembershipEvent::MemberCardChanged {
            pseudonym, display_name, ..
        }) if *pseudonym == bea && *display_name == short(&bea))
    };
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            ann_events.any(card)
        })
        .await,
        "the joiner's unnamed card shows its short fingerprint on the creator"
    );
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.sim.quiesce().await;
    let joined = |named: bool| {
        ann_events.matching(|e| {
            matches!(e, SubscriptionEvent::Membership(MembershipEvent::Joined {
                pseudonym, display_name, ..
            }) if *pseudonym == bea && display_name.is_some() == named)
        })
    };
    assert_eq!(
        (joined(false), joined(true)),
        (1, 0),
        "one Joined, before the card is read, with no name"
    );
    assert_eq!(
        ann_events.matching(card),
        1,
        "the card's name is announced once"
    );
}

/// The short pseudonym fingerprint of `pseudonym_hex` (`"a1b2 c3d4"`).
fn short(pseudonym_hex: &str) -> String {
    let key = hex::decode(pseudonym_hex).expect("a hex pseudonym");
    rekindle_crypto::identity::short_key_fingerprint(&key)
}

fn settings(request: SettingsRequest) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Settings(request))
}

/// `node`'s `myDisplayName` and `myCardName` for the community.
async fn my_names(community: &Community, i: usize) -> (String, Option<String>) {
    let views = community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityViews,
        )))
        .await;
    let view = (views.as_array().expect("views").iter())
        .find(|v| v["id"].as_str() == Some(community.anchor.as_str()))
        .cloned()
        .expect("the community's view");
    (
        view["myDisplayName"].as_str().expect("a name").to_owned(),
        view["myCardName"].as_str().map(str::to_owned),
    )
}

/// Whether any event `events` holds carries an identity's name
/// (`member0`, `member1`, the harness's identities).
fn names_an_identity(events: &Events) -> bool {
    events.any(|e| {
        let json = serde_json::to_string(e).expect("an event");
        json.contains("member0") || json.contains("member1")
    })
}

/// Plan E3.4 S6: the cards written by the origin, the join and a profile
/// update never carry the identity's name. Each member's `myDisplayName`
/// and the other's member row are the short fingerprint, and the custom
/// status reaches the other member.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn no_card_carries_the_identity_name() {
    let mut community = Community::create(2, 722).await;
    let ann_events = community.nodes[0].events().await;
    let bea_events = community.nodes[1].events().await;
    community.join(1).await;
    let (ann, bea) = (community.pseudonym(0).await, community.pseudonym(1).await);
    community.nodes[1]
        .ok(settings(SettingsRequest::CommunityUpdateProfile {
            community: community.anchor.clone(),
            patch: CommunityProfilePatch {
                custom_status: ProfileField::Set("afk".into()),
                ..CommunityProfilePatch::default()
            },
        }))
        .await;
    let status_seen = |e: &SubscriptionEvent| {
        matches!(e, SubscriptionEvent::Membership(MembershipEvent::MemberCardChanged {
            pseudonym, display_name, custom_status, ..
        }) if *pseudonym == bea && *display_name == short(&bea)
            && custom_status.as_deref() == Some("afk"))
    };
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            ann_events.any(status_seen)
        })
        .await,
        "the updated card reaches the creator, unnamed"
    );
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.sim.quiesce().await;
    assert_eq!(my_names(&community, 0).await, (short(&ann), None));
    assert_eq!(my_names(&community, 1).await, (short(&bea), None));
    let row = listed(&community, 0, &bea)
        .await
        .expect("the joiner listed");
    assert_eq!(row["displayName"].as_str(), Some(short(&bea).as_str()));
    assert!(!names_an_identity(&ann_events), "the creator's events");
    assert!(!names_an_identity(&bea_events), "the joiner's events");
}

/// Plan E3.4 S6: a name set for the community is our card's: the other
/// member's events and row show it, our view reports it, and clearing it
/// returns every surface to the short fingerprint.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_set_name_round_trips_through_the_card() {
    let mut community = Community::create(2, 723).await;
    let ann_events = community.nodes[0].events().await;
    community.join(1).await;
    let bea = community.pseudonym(1).await;
    let set = |name: &str| {
        settings(SettingsRequest::CommunitySetDisplayName {
            community: community.anchor.clone(),
            display_name: if name.is_empty() {
                ProfileField::Clear
            } else {
                ProfileField::Set(name.into())
            },
        })
    };
    community.nodes[1].ok(set("Bea")).await;
    assert_eq!(
        my_names(&community, 1).await,
        ("Bea".to_owned(), Some("Bea".to_owned()))
    );
    let named = |name: String| {
        let bea = bea.clone();
        move |e: &SubscriptionEvent| {
            matches!(e, SubscriptionEvent::Membership(MembershipEvent::MemberCardChanged {
                pseudonym, display_name, ..
            }) if *pseudonym == bea && *display_name == name)
        }
    };
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            ann_events.any(named("Bea".into()))
        })
        .await,
        "the name reaches the creator"
    );
    let row = listed(&community, 0, &bea)
        .await
        .expect("the joiner listed");
    assert_eq!(row["displayName"].as_str(), Some("Bea"));
    community.nodes[1].ok(set("")).await;
    assert_eq!(my_names(&community, 1).await, (short(&bea), None));
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            ann_events.any(named(short(&bea)))
        })
        .await,
        "the cleared name reaches the creator as the fingerprint"
    );
}

/// A profile update of `bio` and `custom_status` alone (every other field
/// kept).
fn profile_update(
    community: &Community,
    bio: Option<&str>,
    custom_status: Option<&str>,
) -> IpcRequest {
    settings(SettingsRequest::CommunityUpdateProfile {
        community: community.anchor.clone(),
        patch: CommunityProfilePatch {
            bio: bio.map_or(ProfileField::Unchanged, |value| {
                ProfileField::Set(value.to_owned())
            }),
            custom_status: custom_status.map_or(ProfileField::Unchanged, |value| {
                ProfileField::Set(value.to_owned())
            }),
            ..CommunityProfilePatch::default()
        },
    })
}

/// Read our profile as a newly opened community window sees it.
async fn my_profile(community: &Community, i: usize) -> [Option<String>; 4] {
    let views = community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityViews,
        )))
        .await;
    let view = (views.as_array().expect("views").iter())
        .find(|v| v["id"].as_str() == Some(community.anchor.as_str()))
        .cloned()
        .expect("the community's view");
    ["myBio", "myCustomStatus", "myAvatarRef", "myBannerRef"]
        .map(|field| view[field].as_str().map(str::to_owned))
}

/// Plan E3.4 S6: a status-only update keeps our bio, and our whole card
/// (the name and status cached on our roster row when read back, the
/// profile stored there) is ours at once after a restart, before any
/// record is read.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn our_card_survives_a_restart() {
    let mut community = Community::create(2, 724).await;
    community.join(1).await;
    community.nodes[1]
        .ok(settings(SettingsRequest::CommunitySetDisplayName {
            community: community.anchor.clone(),
            display_name: ProfileField::Set("Bea".into()),
        }))
        .await;
    let avatar = "a".repeat(64);
    let banner = "b".repeat(64);
    community.nodes[1]
        .ok(settings(SettingsRequest::CommunityUpdateProfile {
            community: community.anchor.clone(),
            patch: CommunityProfilePatch {
                bio: ProfileField::Set("hi".into()),
                avatar_ref: ProfileField::Set(avatar.clone()),
                banner_ref: ProfileField::Set(banner.clone()),
                ..CommunityProfilePatch::default()
            },
        }))
        .await;
    community.nodes[1]
        .ok(profile_update(&community, None, Some("afk")))
        .await;
    let mut expected = [
        Some("hi".to_owned()),
        Some("afk".to_owned()),
        Some(avatar),
        Some(banner),
    ];
    assert_eq!(
        my_profile(&community, 1).await,
        expected,
        "status-only keeps the bio and both pictures"
    );
    community.sim.quiesce().await;
    let key = community.nodes[1].public_key.clone().expect("an identity");
    let restarted = community
        .nodes
        .remove(1)
        .restart(&community.sim, Duration::from_secs(10))
        .await;
    community.nodes.insert(1, restarted);
    community.nodes[1]
        .ok(IpcRequest::Unlock {
            identity: key,
            passphrase: PASSPHRASE.into(),
        })
        .await;
    assert_eq!(
        my_names(&community, 1).await,
        ("Bea".to_owned(), Some("Bea".to_owned()))
    );
    assert_eq!(my_profile(&community, 1).await, expected, "after a restart");
    community.nodes[1]
        .ok(settings(SettingsRequest::CommunityUpdateProfile {
            community: community.anchor.clone(),
            patch: CommunityProfilePatch {
                banner_ref: ProfileField::Clear,
                ..CommunityProfilePatch::default()
            },
        }))
        .await;
    expected[3] = None;
    assert_eq!(my_profile(&community, 1).await, expected, "banner cleared");
}
