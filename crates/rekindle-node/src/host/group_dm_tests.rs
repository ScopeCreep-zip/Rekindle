//! The group-DM owner's Host adapter (plan E2.6 G2b) over a real unlocked
//! identity: a created group, its keys and a held invite live in the
//! identity's database, and its events reach the journal.

use rekindle_dm::{GroupDmDeps, PendingInvite};
use rekindle_types::group_dm::GroupDmRosterMember;

use super::adapters::friends::HostFriends;
use super::adapters::group_dm::HostGroupDm;
use super::direct_tests::ann;

fn member(identity: String, n: char) -> GroupDmRosterMember {
    GroupDmRosterMember {
        identity,
        profile_record_key: format!("VLD0:{}", n.to_string().repeat(43)),
        participant_record_key: format!("VLD0:{}", n.to_ascii_uppercase().to_string().repeat(43)),
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
    let created = rekindle_dm::create(&deps, "raid night", own, others)
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
