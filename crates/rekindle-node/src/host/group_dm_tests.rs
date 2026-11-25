//! The group-DM owner's Host adapter (plan E2.6 G2b) over a real unlocked
//! identity: a created group, its keys and a held invite live in the
//! identity's database, and its events reach the journal.

use rekindle_dm::{GroupDmDeps, PendingInvite};
use rekindle_types::group_dm::GroupDmRosterMember;

use super::adapters::friends::HostFriends;
use super::adapters::group_dm::HostGroupDm;
use super::direct_tests::ann;

pub(super) fn member(identity: String, n: char) -> GroupDmRosterMember {
    GroupDmRosterMember {
        identity,
        profile_record_key: format!("VLD0:{}", n.to_string().repeat(43)),
        display_name: format!("member {n}"),
    }
}

#[tokio::test]
async fn a_group_its_keys_and_a_held_invite_are_the_identitys() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let mut events = host.ctx.journal.subscribe(None);
    let deps = HostGroupDm::open(&HostFriends::of(&host.ctx)).unwrap();
    assert_eq!(deps.own_identity(), hex::encode(owner));

    let own = member(hex::encode(owner), 'a');
    let others = vec![member("b".repeat(64), 'b'), member("c".repeat(64), 'c')];
    let created = rekindle_dm::create(
        &deps,
        [7; 16],
        "raid night",
        own,
        others,
        &format!("VLD0:{}", "Q".repeat(43)),
    )
    .await
    .unwrap();
    let gid = created.group.group_id;
    assert_eq!(deps.group(&gid).await.unwrap(), Some(created.group.clone()));
    assert_eq!(deps.groups().await.unwrap().len(), 1);
    let key = deps
        .latest_sender_key(&gid, &deps.own_identity())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*key.as_bytes(), created.invite.frame.creator_sender_key);
    rekindle_dm::group::verify_invite(&created.invite.frame, &deps.own_identity())
        .expect("the invite is signed by the identity");

    let pending = PendingInvite {
        from: "b".repeat(64),
        invite: created.invite.frame.clone(),
    };
    deps.stash_invite(&pending).await.unwrap();
    assert_eq!(deps.take_invite(&gid).await.unwrap(), Some(pending));
    assert_eq!(deps.take_invite(&gid).await.unwrap(), None, "taken once");

    let rotated = rekindle_dm::rotate_own_key(&deps, &gid).await.unwrap();
    assert_eq!(rotated.frame.epoch, key.epoch() + 1);
    assert_eq!(
        deps.group(&gid).await.unwrap().unwrap().epoch,
        key.epoch() + 1
    );
    deps.delete_group(&gid).await.unwrap();
    assert_eq!(deps.group(&gid).await.unwrap(), None);

    let mut seen = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        seen.push(format!("{frame:?}"));
    }
    assert!(seen.iter().any(|f| f.contains("Joined")), "{seen:?}");
    assert!(seen.iter().any(|f| f.contains("KeyRotated")), "{seen:?}");
}

#[tokio::test]
async fn a_member_who_is_not_a_friend_is_admitted_for_the_group_kinds_only() {
    use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId, GroupDmLeave};
    use rekindle_transport::operations::group_dm::admitted;

    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let deps = HostGroupDm::open(&friends).unwrap();
    let (b, c, stranger) = ("b".repeat(64), "c".repeat(64), "d".repeat(64));
    let others = vec![member(b.clone(), 'b'), member(c.clone(), 'c')];
    let created = rekindle_dm::create(
        &deps,
        [7; 16],
        "raid night",
        member(hex::encode(owner), 'a'),
        others,
        &format!("VLD0:{}", "Q".repeat(43)),
    )
    .await
    .unwrap();
    let gid = created.group.group_id;

    for kind in [
        DmTypeId::GroupDmAccept,
        DmTypeId::GroupContent,
        DmTypeId::GroupDmLeave,
    ] {
        assert!(admitted(&friends, &b, kind, false).await, "{kind:?}");
        assert!(
            admitted(&friends, &b, kind, true).await,
            "{kind:?} opens the pair's session"
        );
        assert!(
            !admitted(&friends, &stranger, kind, false).await,
            "{kind:?}"
        );
    }
    for kind in [
        DmTypeId::DmText,
        DmTypeId::FriendAccept,
        DmTypeId::FriendsKey,
    ] {
        assert!(!admitted(&friends, &b, kind, false).await, "{kind:?}");
        assert!(!admitted(&friends, &b, kind, true).await, "{kind:?}");
    }

    // A group's StreamRecord: admitted before it opens, routed to the group path.
    assert!(admitted(&friends, &b, DmTypeId::StreamRecord, false).await);
    assert!(!admitted(&friends, &stranger, DmTypeId::StreamRecord, false).await);

    // b leaves: off our roster, its admission ends, and our key rotates.
    let leave = DirectBody::GroupDmLeave(GroupDmLeave {
        group_id: gid,
        epoch: 0,
    });
    assert!(rekindle_transport::operations::group_dm::apply(&friends, &b, &leave).await);
    let held = deps.group(&gid).await.unwrap().unwrap();
    assert!(!held.roster.contains(&b));
    assert_eq!(held.epoch, created.group.epoch + 1);
    assert!(deps.latest_sender_key(&gid, &b).await.unwrap().is_none());
    assert!(!admitted(&friends, &b, DmTypeId::GroupContent, false).await);
    assert!(admitted(&friends, &c, DmTypeId::GroupContent, false).await);
}

/// Plan E2.6 GC1: the Host labels a group's members, never by a key: a
/// friend by its friend row's name (over the roster's), a non-friend by
/// the name the signed roster carries, marked as not a friend's, and our
/// own entry as "You".
#[tokio::test]
async fn a_groups_members_are_labelled_by_name_never_by_key() {
    use rekindle_types::friend::{FriendState, FriendshipState};

    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let deps = HostGroupDm::open(&friends).unwrap();
    let (me, b, c) = (hex::encode(owner), "b".repeat(64), "c".repeat(64));
    host.ctx.session.write().as_mut().unwrap().friends.insert(
        b.clone(),
        FriendState::new(b.clone(), "Bob".into(), FriendshipState::Accepted),
    );
    rekindle_dm::create(
        &deps,
        [9; 16],
        "raid night",
        member(me.clone(), 'a'),
        vec![member(b.clone(), 'b'), member(c.clone(), 'c')],
        &format!("VLD0:{}", "Q".repeat(43)),
    )
    .await
    .unwrap();

    let views = rekindle_transport::operations::group_dm::list(&friends)
        .await
        .unwrap();
    let labels: Vec<(String, String)> = views[0]
        .members
        .iter()
        .map(|m| (m.key.clone(), m.label.clone()))
        .collect();
    assert_eq!(
        labels,
        vec![
            (me, "You".to_owned()),
            (b, "Bob".to_owned()),
            (c, "member c \u{b7} cccc cccc (not a friend)".to_owned()),
        ]
    );
}

/// GN1: a received invite's members, in its `Invited` event and in the
/// list: a friend by our friend row's name, ourselves as "You", a
/// non-friend by the name this roster carries with their short key
/// fingerprint and the not-a-friend marker, and a non-friend the roster
/// names nobody by that fingerprint alone; never "Unknown".
#[tokio::test]
async fn an_invites_non_friend_members_are_served_by_fingerprint_never_unknown() {
    use rekindle_types::friend::{FriendState, FriendshipState};

    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (host_a, owner_a) = ann(&dir_a).await;
    let (host_b, owner_b) = ann(&dir_b).await;
    let creator = HostGroupDm::open(&HostFriends::of(&host_a.ctx)).unwrap();
    let friends_b = HostFriends::of(&host_b.ctx);
    let deps_b = HostGroupDm::open(&friends_b).unwrap();
    let (a, b) = (hex::encode(owner_a), hex::encode(owner_b));
    let (c, d) = ("c".repeat(64), "d".repeat(64));
    host_b.ctx.session.write().as_mut().unwrap().friends.insert(
        a.clone(),
        FriendState::new(a.clone(), "Ann".into(), FriendshipState::Accepted),
    );
    let mut cara = member(c.clone(), 'c');
    cara.display_name = "Cara".into();
    let mut unnamed = member(d.clone(), 'd');
    unnamed.display_name = String::new();
    let created = rekindle_dm::create(
        &creator,
        [5; 16],
        "raid night",
        member(a.clone(), 'a'),
        vec![member(b.clone(), 'b'), cara, unnamed],
        &format!("VLD0:{}", "Q".repeat(43)),
    )
    .await
    .unwrap();

    let mut events = host_b.ctx.journal.subscribe(None);
    rekindle_dm::on_invite(&deps_b, &a, created.invite.frame)
        .await
        .unwrap();
    let want = [
        (a.clone(), "Ann".to_owned()),
        (b.clone(), "You".to_owned()),
        (c.clone(), "Cara \u{b7} cccc cccc (not a friend)".to_owned()),
        (d.clone(), "dddd dddd (not a friend)".to_owned()),
    ];
    let frame = tokio::time::timeout(std::time::Duration::from_secs(2), events.next())
        .await
        .expect("the invite is published")
        .expect("a journal frame");
    let seen = format!("{frame:?}");
    assert!(seen.contains("Invited"), "{seen}");
    assert!(!seen.contains("Unknown"), "{seen}");
    for (key, label) in &want {
        let row = format!("public_key: {key:?}, display_name: {label:?}");
        assert!(seen.contains(&row), "{row} not in {seen}");
    }

    let views = rekindle_transport::operations::group_dm::list(&friends_b)
        .await
        .unwrap();
    let labels: Vec<(String, String)> = views[0]
        .members
        .iter()
        .map(|m| (m.key.clone(), m.label.clone()))
        .collect();
    assert!(views[0].pending);
    assert_eq!(labels, want.to_vec());
}
