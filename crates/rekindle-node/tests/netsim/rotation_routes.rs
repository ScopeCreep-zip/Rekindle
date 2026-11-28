//! Plan E3.4 S4, routes for a key rotation: the recipients of a rotated
//! community key are every roster member with an `M`/6 route, taken from
//! the rotator's member route cache, and the rotator reads every route
//! it has not cached once before it distributes. After an unlock the
//! cache holds only the watch ring's routes (`RING_K` = 4), so in a
//! community of seven the rotator's ring leaves members out; a kick's
//! rotation must still reach each of them by its `M`/6 route directly,
//! with no member having to ask for the key (`RequestMEK`).
//!
//! Fails without the pre-fill (`HostGovernance::fill_member_routes` in
//! `online_recipients`): the members outside the ring are not sent the
//! key, and each asks for it (a `RequestMEK` the rotator answers).

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};

use crate::harness::{eventually, Community, PASSPHRASE};

const ANSWERED: &str = "RequestMEK answered";

/// The community key generation node `i` holds, or 0 before it holds one.
async fn community_generation(community: &Community, i: usize) -> u64 {
    community
        .mek_list(i)
        .await
        .as_array()
        .into_iter()
        .flatten()
        .filter(|k| k["channel_id"].as_str().is_none_or(str::is_empty))
        .filter_map(|k| k["generation"].as_u64())
        .max()
        .unwrap_or(0)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_rotation_after_an_unlock_reaches_members_outside_the_ring_by_m6() {
    let community = Community::of(7, 4420).await;
    tokio::time::sleep(Duration::from_secs(120)).await;
    let rotator = &community.nodes[0];
    let key = rotator.public_key.clone().expect("an identity");
    rotator.ok(IpcRequest::Lock).await;
    rotator
        .ok(IpcRequest::Unlock {
            identity: key,
            passphrase: PASSPHRASE.into(),
        })
        .await;
    let cached = || {
        rotator
            .host
            .governance()
            .routed_members(&community.anchor)
            .len()
    };
    assert!(
        eventually(&community.sim, Duration::from_secs(120), || async {
            cached() > 0
        })
        .await,
        "the rotator's ring fills its cache after the unlock"
    );
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(
        cached() < 6,
        "the precondition: the rotator's cache misses members outside its ring"
    );
    let kicked = community.pseudonym(1).await;
    let remaining = [2, 3, 4, 5, 6];
    let mut before = Vec::new();
    for i in remaining {
        before.push(community_generation(&community, i).await);
    }
    let answered_before: usize = (0..7)
        .map(|i| crate::logs::logged(i, &[ANSWERED]).len())
        .sum();
    rotator
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.anchor.clone(),
                target_pseudonym: kicked,
                reason: None,
            },
        )))
        .await;
    for (n, i) in remaining.into_iter().enumerate() {
        assert!(
            eventually(&community.sim, Duration::from_secs(180), || async {
                community_generation(&community, i).await > before[n]
            })
            .await,
            "node {i} receives the rotated key"
        );
    }
    let answered: usize = (0..7)
        .map(|i| crate::logs::logged(i, &[ANSWERED]).len())
        .sum();
    assert_eq!(
        answered, answered_before,
        "every remaining member got the key from the rotation itself, none by asking"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn rotation_routes_reconciles_the_joiners_first_write() {
    use rekindle_governance_runtime::member_routes::{read_member_route, RoutePin};
    use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
    use rekindle_netsim::{Fault, Records, Writes};
    use rekindle_records::port::RouteClass;
    use rekindle_types::dht_layout::member_record::{CARD, ROUTES, SUBKEY_COUNT};

    let mut community = Community::create(2, 4421).await;
    let link = community.invite().await;
    let joiner = &community.nodes[1];
    let governance = joiner.host.governance();
    let old = joiner.routes.own_blob(RouteClass::General).unwrap();
    community.sim.fault(Fault::WriteStalls {
        node: 1,
        of: Writes::Subkey {
            subkey: CARD,
            record: Some(Records::CreatedDflt(SUBKEY_COUNT)),
        },
        stall: Duration::from_secs(12),
    });
    let rotate = async {
        let own_binding = || {
            let p = governance.my_pseudonym(&community.anchor)?;
            let state = governance.governance_state(&community.anchor)?;
            let member = state.members.get(&p)?;
            Some((
                p,
                member.member_record.clone(),
                member.member_record_owner.clone(),
            ))
        };
        assert!(
            eventually(&community.sim, Duration::from_secs(60), || async {
                own_binding()
                    .is_some_and(|(_, record, _)| community.sim.is_setting(1, &record, CARD))
            })
            .await,
            "the join took its route snapshot and is publishing the first card"
        );
        let (p, record, owner) = own_binding().unwrap();
        assert!(community.sim.network_get(&record, ROUTES).is_none());
        joiner.routes.replace_own_class(RouteClass::General);
        assert!(
            eventually(&community.sim, Duration::from_secs(10), || async {
                joiner
                    .routes
                    .own_blob(RouteClass::General)
                    .is_some_and(|current| current != old)
            })
            .await,
            "the new route is available before the stalled card write finishes"
        );
        assert!(community.sim.is_setting(1, &record, CARD));
        (p, record, owner)
    };
    let ((), (p, record, owner)) = tokio::join!(community.join_with(1, link), rotate);
    let port = community.nodes[0].host.governance().record_port().unwrap();
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            let current = joiner.routes.own_blob(RouteClass::General).unwrap();
            read_member_route(
                &*port,
                &community.anchor,
                &record,
                &p,
                RoutePin::Joined(&owner),
            )
            .await
            .is_ok_and(|route| route == Some(current))
        })
        .await,
        "a reader sees the joiner's current General route in M/6"
    );
}
