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

/// The key catch-up (plan E3.1 C2b; register S7l HIGH-1): a community
/// whose key is held behind governance's generation is asked for; one
/// whose key was never held is not (the joiner's first key is the
/// inviter's D16 push), and once the key at that generation is held,
/// nothing is.
#[tokio::test]
async fn recover_behind_meks_asks_only_for_a_held_key_behind() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    {
        let mut map = host.ctx.communities.map().write();
        let c = map.get_mut("c1").unwrap();
        c.mek_generation = 3;
        c.my_pseudonym_key = Some("me".into());
    }
    let governance = host.governance();
    assert!(
        governance.behind_meks().is_empty(),
        "no key held: the joiner waits for the inviter's push"
    );

    let cache = crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
        &host.ctx.mek_cache,
    ));
    let at = |generation| rekindle_transport::crypto::mek::Mek::from_bytes([7; 32], generation);
    rekindle_mek_rotation::ChannelMekCache::insert(&cache, "c1", KeyScope::Community, at(2));
    assert_eq!(
        governance.behind_meks(),
        vec![("c1".into(), "me".into())],
        "generation 2 is behind 3"
    );
    rekindle_mek_rotation::ChannelMekCache::insert(&cache, "c1", KeyScope::Community, at(3));
    assert!(
        governance.behind_meks().is_empty(),
        "the current key is held"
    );
    governance.recover_behind_meks();
    host.shutdown().await.unwrap();
}

/// The sync loop's community phase (plan E3.1 C2b): one pass asks for
/// every channel of the community since our newest message, and marks
/// each request pending.
#[tokio::test]
async fn the_community_phase_requests_every_channel() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    {
        let mut map = host.ctx.communities.map().write();
        let c = map.get_mut("c1").unwrap();
        c.my_pseudonym_key = Some("me".into());
        c.channel_log_keys.insert("ch-a".into(), "rec-a".into());
        c.channel_log_keys.insert("ch-b".into(), "rec-b".into());
    }
    host.governance().sync_community("c1", &[1, 2, 3]).await;
    let pending: std::collections::BTreeSet<String> = host.ctx.communities.map().read()["c1"]
        .pending_syncs
        .keys()
        .cloned()
        .collect();
    assert_eq!(pending, ["ch-a".to_string(), "ch-b".to_string()].into());
    host.shutdown().await.unwrap();
}

/// While Invisible the community phase sends nothing: five minutes of
/// ticks leave no SyncRequest (none is marked pending) and no presence;
/// the first pass after coming back asks for every channel.
#[tokio::test(start_paused = true)]
async fn an_invisible_member_sends_no_sync_request() {
    use rekindle_types::presence::SessionStatus;
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    {
        let mut map = host.ctx.communities.map().write();
        let c = map.get_mut("c1").unwrap();
        c.my_pseudonym_key = Some("me".into());
        c.channel_log_keys.insert("ch-a".into(), "rec-a".into());
    }
    host.ctx
        .own_presence
        .set_status(SessionStatus::Invisible)
        .await;
    for _ in 0..10 {
        host.governance().sync_community("c1", &[1, 2, 3]).await;
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
    assert!(host.ctx.communities.map().read()["c1"]
        .pending_syncs
        .is_empty());
    host.ctx
        .own_presence
        .set_status(SessionStatus::Online)
        .await;
    host.governance().sync_community("c1", &[1, 2, 3]).await;
    assert!(host.ctx.communities.map().read()["c1"]
        .pending_syncs
        .contains_key("ch-a"));
    host.shutdown().await.unwrap();
}

/// D16 (register S7l, S7 review HIGH-1): the inviter's key for a joiner
/// is a committed `mek_transfer` row (sealed, never the key), one per
/// (joiner, generation), and the joiner's departure discards it.
#[tokio::test]
async fn the_joiner_s_key_is_a_committed_row_until_the_joiner_departs() {
    use rekindle_governance::op::OpStamp;
    use rekindle_types::id::PseudonymKey;

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let joiner = hex::encode(
        rekindle_crypto::group::pseudonym::derive_community_pseudonym(&[5; 32], "c1")
            .verifying_key()
            .to_bytes(),
    );
    {
        let mut map = host.ctx.communities.map().write();
        let c = map.get_mut("c1").unwrap();
        c.my_pseudonym_key = Some(hex::encode([1; 32]));
        let mut gov = rekindle_governance::state::GovernanceState::default();
        gov.members.insert(
            PseudonymKey::from_hex_lossy(&joiner),
            rekindle_governance::state::MemberState {
                member_record: "r".into(),
                member_record_owner: "o".into(),
                joined: OpStamp::MIN,
            },
        );
        c.governance_state = Some(gov);
    }
    let cache = crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
        &host.ctx.mek_cache,
    ));
    let key = rekindle_transport::crypto::mek::Mek::from_bytes([7; 32], 3);
    rekindle_mek_rotation::ChannelMekCache::insert(&cache, "c1", KeyScope::Community, key);
    let deps = crate::daemon::mek_rotation::DaemonMekAdapter::new(std::sync::Arc::clone(&host.ctx));
    let bootstrap = rekindle_governance_runtime::BootstrapJoiner {
        pseudonym: joiner.clone(),
        member_joined: rekindle_types::governance::OpRef {
            op_hash: OpStamp::MIN.hash,
            author: PseudonymKey::from_hex_lossy(&joiner),
            author_record: "r".into(),
            author_seq: 1,
        },
        member_joined_op: Vec::new(),
        route: b"route".to_vec(),
    };
    let governance = host.governance();
    for _ in 0..2 {
        governance
            .deliver_key_to_joiner(&deps, "c1", &bootstrap)
            .await;
    }
    let rows = || async {
        let owner = owner.clone();
        host.ctx
            .db
            .current()
            .unwrap()
            .call(move |c| rekindle_db::repo::outbox::prefix_rows(c, &owner, "mek_transfer:c1:"))
            .await
            .unwrap()
    };
    let queued = rows().await;
    assert_eq!(queued.len(), 1, "one row per joiner and generation");
    assert_eq!(queued[0].stream, format!("mek_transfer:c1:{joiner}"));
    assert_eq!(queued[0].client_id, format!("MekTransfer:c1:{joiner}:3"));
    assert!(
        !queued[0].payload.windows(32).any(|w| w == [7; 32]),
        "the row holds the sealed key, never the key"
    );

    governance.forget_member("c1", &joiner);
    let mut left = queued.len();
    for _ in 0..50 {
        left = rows().await.len();
        if left == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(left, 0, "the joiner's departure discards its row");
    host.shutdown().await.unwrap();
}
