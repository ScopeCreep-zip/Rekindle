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
            device_key: PseudonymKey::from_hex_lossy(&joiner).0,
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

/// FX3 d (register `audit-2026-10-23` FX3; Q6): the elected `RequestMEK`
/// responder hands a restricted channel's key only to a member of that
/// channel's read roster. A member the channel's `@everyone` overwrite
/// shuts out is refused; one a role overwrite lets in passes the gate.
#[tokio::test]
async fn the_responder_refuses_a_restricted_channel_key_outside_its_roster() {
    use std::collections::HashSet;

    use rekindle_governance::state::{GovernanceState, MemberState, OverwriteState, RoleState};
    use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
    use rekindle_types::permissions::{DEFAULT_EVERYONE, VIEW_CHANNELS};

    use crate::host::adapters::governance::MekRequest;

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let channel = ChannelId([9; 16]);
    let outsider = PseudonymKey([2; 32]);
    let insider = PseudonymKey([3; 32]);
    let reader = RoleId([5; 16]);
    let role = |permissions| RoleState {
        name: "r".into(),
        permissions,
        position: 1,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
        stamp: rekindle_governance::fixture::stamp(1),
    };
    let overwrite = |allow, deny| OverwriteState {
        target_type: "role".into(),
        allow,
        deny,
        stamp: rekindle_governance::fixture::stamp(2),
    };
    let mut gov = GovernanceState::default();
    gov.roles.insert(RoleId([0; 16]), role(DEFAULT_EVERYONE));
    gov.roles.insert(reader, role(DEFAULT_EVERYONE));
    gov.overwrites.insert(
        (channel, format!("{:?}", RoleId([0; 16]))),
        overwrite(0, VIEW_CHANNELS),
    );
    gov.overwrites.insert(
        (channel, format!("{reader:?}")),
        overwrite(VIEW_CHANNELS, 0),
    );
    gov.role_assignments
        .insert(insider.clone(), HashSet::from([reader]));
    for member in [&outsider, &insider] {
        gov.members.insert(
            member.clone(),
            MemberState {
                member_record: "r".into(),
                member_record_owner: "o".into(),
                joined: rekindle_governance::op::OpStamp::MIN,
            },
        );
    }
    {
        let mut map = host.ctx.communities.map().write();
        let c = map.get_mut("c1").unwrap();
        c.my_pseudonym_key = Some(hex::encode([1; 32]));
        c.governance_state = Some(gov);
    }
    let deps = crate::daemon::mek_rotation::DaemonMekAdapter::new(std::sync::Arc::clone(&host.ctx));
    let governance = host.governance();
    let ask = |requester: &PseudonymKey| MekRequest {
        channel_id: KeyScope::Channel(channel).wire_channel_str(),
        needed_generation: 0,
        requester_pseudonym: hex::encode(requester.0),
        cascade_index: 0,
    };
    let answer = |request: MekRequest| {
        let governance = governance.clone();
        let deps = &deps;
        async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(20),
                governance.answer_mek_request(deps, "c1", &request),
            )
            .await
        }
    };
    let refused = answer(ask(&outsider)).await;
    assert!(
        matches!(&refused, Ok(Err(e)) if e.contains("restricted channel's roster")),
        "an outsider is refused the restricted channel's key: {refused:?}"
    );
    let passed = answer(ask(&insider)).await;
    assert!(
        !matches!(&passed, Ok(Err(e)) if e.contains("restricted channel's roster")),
        "a roster member passes the gate: {passed:?}"
    );
    assert!(governance.reads_scope("c1", KeyScope::Community, &outsider));
    assert!(governance.reads_scope("c1", KeyScope::Channel(ChannelId([8; 16])), &outsider));
    host.shutdown().await.unwrap();
}

/// KP: a member whose community key is behind governance's generation
/// reads `Pending` with what it holds and needs (and since when), and
/// `Current` once the key at that generation is installed.
#[tokio::test]
async fn a_member_behind_reads_pending_until_the_key_arrives() {
    use rekindle_types::dto::community::KeyState;
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
    let cache = crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
        &host.ctx.mek_cache,
    ));
    let at = |generation| rekindle_transport::crypto::mek::Mek::from_bytes([7; 32], generation);
    rekindle_mek_rotation::ChannelMekCache::insert(&cache, "c1", KeyScope::Community, at(2));
    governance.refresh_key_status("c1");
    let status = |views: Vec<rekindle_types::dto::community::CommunityView>| {
        views.into_iter().find(|v| v.id == "c1").unwrap().key_status
    };
    let pending = status(governance.community_views().await);
    assert_eq!(pending.state, KeyState::Pending, "generation 2 is behind 3");
    assert_eq!((pending.have, pending.need), (Some(2), Some(3)));
    assert!(pending.since_ms.is_some(), "the wait has a start");

    rekindle_mek_rotation::ChannelMekCache::insert(&cache, "c1", KeyScope::Community, at(3));
    governance.refresh_key_status("c1");
    let current = status(governance.community_views().await);
    assert_eq!(current.state, KeyState::Current, "the key arrived");
    assert_eq!((current.have, current.since_ms), (Some(3), None));
    host.shutdown().await.unwrap();
}

/// KP: a member outside a channel's readers sees it locked with the
/// Host's reason; a creator, who reads everything, does not.
#[test]
fn a_channel_we_cannot_read_is_locked_with_the_host_s_reason() {
    use crate::host::adapters::governance::views::channel_lock;
    use rekindle_types::id::PseudonymKey;
    let creator = PseudonymKey([1; 32]);
    let c = |me: u8| crate::host::community::CommunityState {
        my_pseudonym_key: Some(hex::encode([me; 32])),
        governance_state: Some(rekindle_governance::state::GovernanceState {
            creators: std::collections::BTreeSet::from([creator.clone()]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let channel = hex::encode([9u8; 16]);
    let reason = channel_lock(&c(2), &channel, 0).expect("a non-reader sees the channel locked");
    assert_eq!(reason, "You don't have permission to view this channel.");
    assert_eq!(
        channel_lock(&c(1), &channel, 0),
        None,
        "the creator reads it"
    );
}
