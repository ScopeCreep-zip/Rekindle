use parking_lot::RwLock;
use rekindle_transport::session::{CommunityMembership, Session};

use super::*;
use crate::host::fixture::Fixture;
use crate::host::identity;

fn membership() -> CommunityMembership {
    CommunityMembership {
        governance_key: "gov-key".into(),
        pseudonym_key: "cd".repeat(32),
        role_ids: vec![0, 7],
        registry_key: "reg-key".into(),
        slot_index: 3,
        community_name: "Team".into(),
        slot_seed: Some([9; 32]),
        channel_record_keys: std::collections::HashMap::new(),
        segment_index: Some(1),
        lamport_counter: 12,
        mek_generation: 2,
    }
}

/// Everything the daemon writes through these helpers comes back in the
/// projection the next unlock loads: the identity, a membership with its
/// slot seed and clock, a friend, and a forgotten membership's absence.
#[tokio::test]
async fn writes_come_back_in_the_next_projection() {
    let f = Fixture::new();
    let made = identity::create(
        &f.root,
        &f.stores(),
        Some("Ann"),
        "pass",
        1,
        &rekindle_utils::random::OsRandom,
    )
    .await
    .unwrap();
    let owner = made.public_key.clone();
    drop(made);
    let db = f.db.current().unwrap();
    let session = RwLock::new(Some(
        load(&db, &f.vault, &owner, &rekindle_utils::OsRandom, 0)
            .await
            .unwrap(),
    ));
    assert_eq!(
        session.read().as_ref().unwrap().identity.display_name,
        "Ann"
    );

    let communities =
        crate::host::community::CommunityStore::new(crate::host::community::store::new_map());
    record_membership(
        &session,
        &f.db,
        &f.vault,
        &communities,
        NewMembership {
            membership: membership(),
            dht_owner_keypair: None,
            registry_keypair: Some("reg-kp".into()),
            member_record_owner: Some(zeroize::Zeroizing::new("mr-kp".into())),
            member_hpke_secret: Some(zeroize::Zeroizing::new([7; 32])),
            genesis_cert: None,
            joined_at: 1,
        },
    )
    .unwrap();
    // `K_M(p)` is in the vault (plan E3.2, D1: before the row).
    let member_record_owner = f
        .vault
        .lock()
        .as_ref()
        .unwrap()
        .get(&rekindle_vault::VaultKey::MemberRecordOwner {
            community: "gov-key".into(),
        })
        .unwrap()
        .unwrap();
    assert_eq!(member_record_owner.as_slice(), b"mr-kp");
    update_roles(&session, &f.db, &communities, "gov-key", |roles| {
        roles.push(9);
    });
    // The community store holds what the governance adapter reads (B2b).
    {
        let map = communities.map().read();
        let stored = map.get("gov-key").expect("the membership is in the store");
        assert_eq!(
            stored.my_pseudonym_key.as_deref(),
            Some("cd".repeat(32).as_str())
        );
        assert_eq!(stored.governance_clock, 12);
        assert_eq!(stored.my_role_ids, vec![0, 7, 9]);
        assert_eq!(stored.my_subkey_index, Some(3));
        assert_eq!(stored.my_segment_index, Some(1));
        assert_eq!(stored.slot_seed.as_deref(), Some("09".repeat(32).as_str()));
        assert_eq!(stored.registry_owner_keypair.as_deref(), Some("reg-kp"));
    }
    let o = owner.clone();
    db.call(move |conn| {
        rekindle_db::repo::friends::replace_with_invited(
            conn,
            &o,
            "bob",
            "Bob",
            5,
            "bob-profile",
            "bob-mailbox",
        )
    })
    .await
    .unwrap();

    let reloaded: Session = load(&db, &f.vault, &owner, &rekindle_utils::OsRandom, 0)
        .await
        .unwrap();
    // The memberships are the community store's projection (plan E3.1 C1).
    let stored = crate::host::adapters::governance::loader::load(&db, &f.vault, &owner, &[1; 32])
        .await
        .unwrap();
    let m = crate::host::adapters::governance::loader::membership(&stored["gov-key"]);
    assert_eq!(m.slot_seed, Some([9; 32]));
    assert_eq!(m.slot_index, 3);
    assert_eq!(m.segment_index, Some(1));
    assert_eq!(m.lamport_counter, 12);
    assert_eq!(m.role_ids, vec![0, 7, 9]);
    assert_eq!(m.community_name, "Team");
    let bob = &reloaded.friends["bob"];
    assert_eq!(bob.mailbox_dht_key.as_deref(), Some("bob-mailbox"));
    assert_eq!(bob.dht_record_key.as_deref(), Some("bob-profile"));
    assert_eq!(
        bob.friendship_state,
        rekindle_types::friend::FriendshipState::PendingOut
    );

    // Plan E2.5: the first unlock minted our presence key; a reload keeps it.
    let minted = reloaded.presence_key.clone().unwrap();
    assert_eq!(minted.epoch(), 1);

    forget_membership(&session, &f.db, &f.vault, "gov-key")
        .await
        .unwrap();
    let after = load(&db, &f.vault, &owner, &rekindle_utils::OsRandom, 0)
        .await
        .unwrap();
    let stored = crate::host::adapters::governance::loader::load(&db, &f.vault, &owner, &[1; 32])
        .await
        .unwrap();
    assert!(stored.is_empty());
    assert_eq!(after.presence_key, Some(minted));
}

/// The session an unlock loads names no own record, even when the identity
/// row does: only the own-records stage names each, once it holds the record
/// writable (live finding 24), so the route publisher and the status
/// heartbeat never write one first.
#[tokio::test]
async fn the_session_names_no_own_record_before_the_stage_holds_it() {
    use rekindle_db::repo::identity::{set_mailbox_key, set_owned_record, OwnedRecord};
    let f = Fixture::new();
    let made = identity::create(
        &f.root,
        &f.stores(),
        Some("Ann"),
        "pass",
        1,
        &rekindle_utils::random::OsRandom,
    )
    .await
    .unwrap();
    let owner = made.public_key.clone();
    drop(made);
    let db = f.db.current().unwrap();
    let o = owner.clone();
    db.call(move |conn| {
        set_owned_record(conn, &o, OwnedRecord::Profile, "VLD0:profile")?;
        set_owned_record(conn, &o, OwnedRecord::FriendList, "VLD0:friends")?;
        set_mailbox_key(conn, &o, "VLD0:mailbox")?;
        Ok(())
    })
    .await
    .unwrap();
    let session = load(&db, &f.vault, &owner, &rekindle_utils::OsRandom, 0)
        .await
        .unwrap();
    assert_eq!(session.identity.profile_dht_key, "");
    assert_eq!(session.identity.mailbox_dht_key, "");
    assert_eq!(session.identity.friend_list_dht_key, "");
}
