//! The membership control messages the Host handles on both hosts (plan
//! E3.1 B2e), offline: a JoinAccepted and a peer-assisted join.

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_codec::community::types::MemberSummary;
use rekindle_types::channel_keys::KeyScope;

use super::{create_ann, offline_host, seed_registry_community};

fn member(pseudonym: &str, name: &str, subkey_index: u32) -> MemberSummary {
    MemberSummary {
        pseudonym_key: pseudonym.into(),
        display_name: name.into(),
        role_ids: vec![0],
        joined_at: 1,
        subkey_index,
        onboarding_complete: false,
        timeout_until: None,
    }
}

/// A JoinAccepted installs the community key, the MEK generation and the
/// member registry, and stores and knows the members it lists.
#[tokio::test]
async fn a_join_accepted_installs_the_key_registry_and_members() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;

    let key = rekindle_transport::crypto::mek::Mek::from_bytes([7; 32], 3);
    let payload = ControlPayload::JoinAccepted {
        mek_encrypted: key.to_wire_bytes(),
        mek_generation: 3,
        members: vec![member("bob", "Bob", 1), member("cat", "Cat", 2)],
        member_registry_key: Some("registry-2".into()),
        slot_index: None,
        wrapped_slot_seed: None,
    };
    let handled = host
        .governance()
        .on_membership_control("c1", "bob", &payload)
        .await;
    assert!(handled);

    {
        let map = host.ctx.communities.map().read();
        let c = &map["c1"];
        assert_eq!(c.mek_generation, 3);
        assert_eq!(c.member_registry_key.as_deref(), Some("registry-2"));
        assert!(c.known_members.contains("bob") && c.known_members.contains("cat"));
    }
    let cached = host
        .ctx
        .mek_cache
        .read()
        .current("c1", KeyScope::Community)
        .map(rekindle_transport::crypto::mek::Mek::generation);
    assert_eq!(cached, Some(3));
    let names = host
        .ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            Ok((
                rekindle_db::repo::members::display_name(conn, &owner, "c1", "bob")?,
                rekindle_db::repo::members::display_name(conn, &owner, "c1", "cat")?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(names, (Some("Bob".into()), Some("Cat".into())));
    host.shutdown().await.unwrap();
}

/// A peer-assisted join makes the joiner known and counts a use of our
/// invite it redeemed.
#[tokio::test]
async fn a_peer_join_is_known_and_counts_our_invite() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let db = host.ctx.db.current().unwrap();
    let hash = rekindle_secrets::invite::hash_invite_code("CODE");
    let (seeded, seeded_hash) = (owner.clone(), hash.clone());
    db.call(move |conn| {
        rekindle_db::repo::community_invites::insert(conn, &seeded, "c1", "CODE", &seeded_hash, 1)
    })
    .await
    .unwrap();

    let payload = ControlPayload::MemberJoinRequest {
        pseudonym_key: "dan".into(),
        display_name: "Dan".into(),
        invite_code: Some("CODE".into()),
        route_blob: None,
        prekey_bundle: None,
        claimed_subkey_index: Some(3),
    };
    assert!(
        host.governance()
            .on_membership_control("c1", "dan", &payload)
            .await
    );
    assert!(host.ctx.communities.known_members("c1").contains("dan"));

    let mut uses = None;
    for _ in 0..100 {
        let (o, h) = (owner.clone(), hash.clone());
        uses = db
            .call(move |conn| rekindle_db::repo::community_invites::uses(conn, &o, "c1", &h))
            .await
            .unwrap();
        if uses == Some(1) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(uses, Some(1));
    host.shutdown().await.unwrap();
}
