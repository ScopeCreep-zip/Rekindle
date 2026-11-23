//! The Host's governance projection (plan E3.1 B2c).

use super::*;

/// A rebuild result projects merged channels and roles into the community
/// store and the community's SQLite rows (plan E3.1 B2c), the same body on
/// both hosts.
#[tokio::test]
async fn a_rebuild_result_projects_channels_and_roles_into_the_store_and_sqlite() {
    use rekindle_governance::state::{ChannelState, GovernanceState, RoleState};
    use rekindle_governance_runtime::GovernanceRuntimeDeps;
    use rekindle_types::id::{ChannelId, RoleId};

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    let db = host.ctx.db.current().unwrap();
    let seeded = owner.clone();
    db.call(move |conn| {
        rekindle_db::repo::communities::insert_if_absent(
            conn,
            &seeded,
            &rekindle_db::repo::communities::NewCommunity {
                id: "c1",
                name: "C",
                joined_at: 1,
                my_pseudonym_key: "",
                mek_generation: 0,
                member_registry_key: None,
                my_subkey_index: None,
            },
        )
    })
    .await
    .unwrap();

    let channel = |name: &str, position| ChannelState {
        name: name.into(),
        channel_type: "text".into(),
        category_id: None,
        position,
        topic: None,
        forum_tags: None,
        slowmode_seconds: None,
        nsfw: None,
        parent_voice_channel_id: None,
        created: rekindle_governance::fixture::stamp(1),
    };
    let mut gov = GovernanceState::default();
    gov.channels
        .insert(ChannelId([2; 16]), channel("second", 1));
    gov.channels
        .insert(ChannelId([1; 16]), channel("general", 0));
    let admin = RoleId([7; 16]);
    gov.roles.insert(
        admin,
        RoleState {
            name: "Admin".into(),
            permissions: 8,
            position: 1,
            color: 0,
            hoist: true,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            stamp: rekindle_governance::fixture::stamp(1),
        },
    );
    host.governance()
        .apply_governance_rebuild_result("c1", gov)
        .await;

    let projected = host
        .ctx
        .communities
        .map()
        .read()
        .get("c1")
        .cloned()
        .unwrap();
    let names: Vec<_> = projected.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["general", "second"]);
    assert_eq!(projected.roles.len(), 1);
    assert_eq!(projected.roles[0].name, "Admin");
    assert!(projected.governance_state.is_some());

    let load = db
        .call(move |conn| rekindle_db::repo::communities::load::load(conn, &owner))
        .await
        .unwrap();
    let mut stored: Vec<_> = load.channels.iter().map(|c| c.name.clone()).collect();
    stored.sort();
    assert_eq!(stored, ["general", "second"]);
    let roles: Vec<_> = load
        .roles
        .iter()
        .map(|r| (r.role_id, r.name.clone()))
        .collect();
    assert_eq!(roles, [(admin, "Admin".to_string())]);
    host.shutdown().await.unwrap();
}

/// A member the merge adds is on the member list by the time its `Joined`
/// is published (walk B61): a window re-reads the list on `Joined`, and the
/// joiner's gossip and the presence poll that also store its row publish
/// nothing a window re-reads on, so an open window never listed it.
#[tokio::test]
async fn a_merged_join_is_on_the_member_list_its_joined_announces() {
    use rekindle_governance::state::{GovernanceState, MemberState};
    use rekindle_governance_runtime::GovernanceRuntimeDeps;
    use rekindle_types::id::PseudonymKey;

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let governance = host.governance();
    // The first projection is the baseline; the second adds Bea.
    governance
        .apply_governance_rebuild_result("c1", GovernanceState::default())
        .await;
    let bea = PseudonymKey([7; 32]);
    let mut joined = GovernanceState::default();
    joined.members.insert(
        bea.clone(),
        MemberState {
            member_record: "record".into(),
            member_record_owner: "owner".into(),
            joined: rekindle_governance::op::OpStamp::MIN,
        },
    );
    joined.op_log_records.insert(bea.clone(), "log".into());
    let mut events = host.ctx.journal.subscribe(None);
    governance
        .apply_governance_rebuild_result("c1", joined)
        .await;

    let mut announced = false;
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        if let rekindle_types::subscription_events::StreamFrame::Event(s) = frame {
            announced |= matches!(
                s.event,
                rekindle_types::subscription_events::SubscriptionEvent::Membership(
                    rekindle_types::subscription_events::MembershipEvent::Joined { ref pseudonym, .. }
                ) if *pseudonym == bea.to_hex()
            );
        }
    }
    assert!(announced, "the join is published");
    let listed = governance.community_members("c1").await.unwrap();
    assert!(
        listed.iter().any(|m| m.pseudonym_key == bea.to_hex()),
        "the member list holds the joiner"
    );
    host.shutdown().await.unwrap();
}

/// A roster row and a beat of a key the merge never admitted (a joiner
/// past a single-use invite's `max_uses`, finding I1) never put it on the
/// member list: the list is the merged, counted members alone, whatever a
/// gossip announce, a registry read or a presence beat stored.
#[tokio::test]
async fn a_key_the_merge_never_admitted_is_never_on_the_member_list() {
    use rekindle_governance::state::{GovernanceState, MemberState};
    use rekindle_governance_runtime::GovernanceRuntimeDeps;
    use rekindle_types::id::PseudonymKey;

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let (bea, stranger) = (PseudonymKey([7; 32]), PseudonymKey([9; 32]));
    let mut merged = GovernanceState::default();
    merged.members.insert(
        bea.clone(),
        MemberState {
            member_record: "record".into(),
            member_record_owner: "owner".into(),
            joined: rekindle_governance::op::OpStamp::MIN,
        },
    );
    let governance = host.governance();
    governance
        .apply_governance_rebuild_result("c1", merged)
        .await;
    // What a refused joiner's gossip announce and member record store,
    // and its presence beat.
    for key in [bea.to_hex(), stranger.to_hex()] {
        let owner = owner.clone();
        host.ctx
            .db
            .current()
            .unwrap()
            .call(move |conn| {
                rekindle_db::repo::members::insert_if_absent(conn, &owner, "c1", &key, 1)
            })
            .await
            .unwrap();
    }
    host.ctx.communities.note_online_member(
        "c1",
        &stranger.to_hex(),
        crate::host::community::OnlineMember {
            route_blob: vec![1],
            status: "online".into(),
            last_seen: 1,
            ..crate::host::community::OnlineMember::default()
        },
    );

    let listed: Vec<String> = governance
        .community_members("c1")
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.pseudonym_key)
        .collect();
    assert_eq!(listed, [bea.to_hex()], "only the admitted member is listed");
    host.shutdown().await.unwrap();
}

/// A founder never announces a join, so the gossip mesh never holds her
/// online; her beats do (finding F2, plan E3.4 premise check). A joiner's
/// member list shows her as her beats show her: online in her hoisted
/// role's group while she beats, offline once her beat expires.
#[tokio::test]
async fn a_founder_is_listed_as_her_beats_show_her_without_a_join_announce() {
    use rekindle_codec::member_record::{BeatState, PresenceBeat};
    use rekindle_governance::state::{GovernanceState, MemberState, RoleState};
    use rekindle_governance_runtime::GovernanceRuntimeDeps;
    use rekindle_types::dto::community::MemberGroup;
    use rekindle_types::id::{PseudonymKey, RoleId};

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let (founder, joiner) = (PseudonymKey([7; 32]), PseudonymKey([0xbb; 32]));
    let owner_role = RoleId([5; 16]);
    let mut merged = GovernanceState::default();
    for key in [&founder, &joiner] {
        merged.members.insert(
            key.clone(),
            MemberState {
                member_record: "record".into(),
                member_record_owner: "owner".into(),
                joined: rekindle_governance::op::OpStamp::MIN,
            },
        );
    }
    merged.roles.insert(
        owner_role,
        RoleState {
            name: "Owner".into(),
            permissions: 8,
            position: 10,
            color: 0,
            hoist: true,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            stamp: rekindle_governance::fixture::stamp(1),
        },
    );
    merged.role_assignments.insert(
        founder.clone(),
        std::collections::HashSet::from([owner_role]),
    );
    let governance = host.governance();
    governance
        .apply_governance_rebuild_result("c1", merged)
        .await;
    host.ctx
        .communities
        .map()
        .write()
        .get_mut("c1")
        .unwrap()
        .my_pseudonym_key = Some(joiner.to_hex());
    for key in [founder.to_hex(), joiner.to_hex()] {
        let owner = owner.clone();
        host.ctx
            .db
            .current()
            .unwrap()
            .call(move |conn| {
                rekindle_db::repo::members::insert_if_absent(conn, &owner, "c1", &key, 1)
            })
            .await
            .unwrap();
    }
    let founder_row = |list: Vec<rekindle_types::dto::community::CommunityMemberView>| {
        list.into_iter()
            .find(|m| m.pseudonym_key == founder.to_hex())
            .map(|m| (m.status, m.group))
            .expect("the founder is listed")
    };

    // Her beat reaches the joiner; no MemberJoined ever did.
    let now = host.ctx.clock.now_ms();
    crate::host::adapters::presence::view::observe(
        &host.ctx,
        "c1",
        &founder,
        &PresenceBeat {
            seq: 1,
            state: BeatState::Online,
            voice_channel: None,
            beat_number: now / rekindle_codec::member_record::beat::BEAT_PERIOD_MS,
            extras: None,
        },
        Some(now),
    );
    assert_eq!(
        founder_row(governance.community_members("c1").await.unwrap()),
        (
            "online".to_owned(),
            MemberGroup::Role {
                id: owner_role.to_hex(),
                name: "Owner".into(),
            }
        ),
        "the founder shows online in her hoisted group"
    );

    // Her beats stop: the 15-minute rule turns her offline.
    let expired = host
        .ctx
        .communities
        .map()
        .write()
        .get_mut("c1")
        .unwrap()
        .beat_view
        .expire(now + 60 * 60 * 1000);
    assert_eq!(expired, [founder.to_hex()]);
    assert_eq!(
        founder_row(governance.community_members("c1").await.unwrap()),
        ("offline".to_owned(), MemberGroup::Offline),
        "the founder shows offline once her beat expires"
    );
    host.shutdown().await.unwrap();
}
