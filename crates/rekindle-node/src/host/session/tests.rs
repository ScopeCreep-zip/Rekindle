use parking_lot::RwLock;
use rekindle_transport::session::{CommunityMembership, PendingFriendRequest, Session};

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
/// slot seed and clock, a pending request, and a forgotten membership's
/// absence.
#[tokio::test]
async fn writes_come_back_in_the_next_projection() {
    let f = Fixture::new();
    let made = identity::create(&f.root, &f.stores(), Some("Ann"), "pass", 1)
        .await
        .unwrap();
    let owner = made.public_key.clone();
    drop(made);
    let db = f.db.current().unwrap();
    let session = RwLock::new(Some(load(&db, &f.vault, &owner).await.unwrap()));
    assert_eq!(
        session.read().as_ref().unwrap().identity.display_name,
        "Ann"
    );

    record_membership(
        &session,
        &f.db,
        &f.vault,
        NewMembership {
            membership: membership(),
            dht_owner_keypair: None,
            registry_keypair: Some("reg-kp".into()),
        },
    )
    .unwrap();
    assert_eq!(next_clock(&session, &f.db, "gov-key").unwrap(), 13);
    update_roles(&session, &f.db, "gov-key", |roles| roles.push(9));
    crate::host::session::record_pending_request(
        &session,
        &f.db,
        PendingFriendRequest {
            public_key: "bob".into(),
            display_name: "Bob".into(),
            message: "hi".into(),
            profile_dht_key: "bob-profile".into(),
            route_blob: vec![1],
            mailbox_dht_key: "bob-mailbox".into(),
            prekey_bundle: vec![2],
            invite_id: None,
            received_at: 5,
        },
    );

    let reloaded: Session = load(&db, &f.vault, &owner).await.unwrap();
    let m = reloaded.community("gov-key").unwrap();
    assert_eq!(m.slot_seed, Some([9; 32]));
    assert_eq!(m.slot_index, 3);
    assert_eq!(m.segment_index, Some(1));
    assert_eq!(m.lamport_counter, 13);
    assert_eq!(m.role_ids, vec![0, 7, 9]);
    assert_eq!(m.community_name, "Team");
    let pending = &reloaded.pending_friend_requests;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].mailbox_dht_key, "bob-mailbox");
    assert_eq!(pending[0].route_blob, vec![1]);

    forget_membership(&session, &f.db, &f.vault, "gov-key")
        .await
        .unwrap();
    let after = load(&db, &f.vault, &owner).await.unwrap();
    assert!(after.communities.is_empty());
}
