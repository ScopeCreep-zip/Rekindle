//! The membership side of the Host on both hosts, offline: the key
//! catch-up, the sync loop's community phase and the joiner's key row.

use rekindle_types::channel_keys::KeyScope;

use super::{create_ann, offline_host, seed_registry_community};

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
        vec![("c1".into(), "me".into(), 3)],
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

/// Item 1c's trigger (plan E3.2 S7m): a merge that raises the key's
/// generation asks for the key; the first projection, an unchanged one and
/// a lower one do not.
#[test]
fn only_a_raised_generation_triggers_the_key_repair() {
    use crate::host::adapters::governance::state_mutations::raises_key_generation;
    use rekindle_governance::state::GovernanceState;
    let at = |mek_generation| GovernanceState {
        mek_generation,
        ..GovernanceState::default()
    };
    assert!(raises_key_generation(Some(&at(1)), &at(2)));
    assert!(!raises_key_generation(None, &at(2)), "a first projection");
    assert!(!raises_key_generation(Some(&at(2)), &at(2)), "unchanged");
    assert!(!raises_key_generation(Some(&at(3)), &at(2)), "lower");
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

    governance.forget_member("c1", &joiner).await;
    assert!(
        rows().await.is_empty(),
        "the joiner's departure discards its row"
    );
    host.shutdown().await.unwrap();
}

/// The raid alert (architecture §20.6) rests on merged joins (plan E3.2
/// S8): with a policy of three joins a minute, the first two members the
/// merge admits raise nothing and the third raises one `RaidDetected`, on
/// both modes, with no gossip notice involved.
#[tokio::test]
async fn merged_joins_past_the_policy_raise_a_raid_alert() {
    use rekindle_governance::state::{CommunityPolicyState, GovernanceState, MemberState};
    use rekindle_governance_runtime::GovernanceRuntimeDeps;
    use rekindle_types::id::PseudonymKey;
    use rekindle_types::subscription_events::{StreamFrame, SubscriptionEvent, SystemEvent};

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let mut state = GovernanceState {
        community_policy: Some(CommunityPolicyState {
            policy_text: None,
            max_joins_per_interval: 3,
            join_interval_seconds: 60,
            stamp: rekindle_governance::fixture::stamp(1),
        }),
        ..GovernanceState::default()
    };
    let governance = host.governance();
    // The first projection is the baseline and announces nothing.
    governance.set_governance_state("c1", state.clone());
    let mut events = host.ctx.journal.subscribe(None);
    let mut raids = Vec::new();
    for m in 1..=3u8 {
        state.members.insert(
            PseudonymKey([m; 32]),
            MemberState {
                member_record: format!("record-{m}"),
                member_record_owner: "owner".into(),
                joined: rekindle_governance::fixture::stamp(u64::from(m) + 1),
            },
        );
        governance.set_governance_state("c1", state.clone());
        while let Ok(Some(frame)) =
            tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
        {
            if let StreamFrame::Event(s) = frame {
                if let SubscriptionEvent::System(SystemEvent::RaidDetected {
                    community,
                    joins_in_window,
                    ..
                }) = s.event
                {
                    raids.push((m, community, joins_in_window));
                }
            }
        }
    }
    assert_eq!(
        raids,
        vec![(3, "c1".to_string(), 3)],
        "the third merged join trips the policy"
    );
    host.shutdown().await.unwrap();
}
