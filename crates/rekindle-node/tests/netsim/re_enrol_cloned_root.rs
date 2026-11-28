//! The original and restored device share one binding until recovery.

use std::time::Duration;

use rekindle_governance_runtime::unlock_barrier::BarrierPhase;
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest, SettingsRequest};
use rekindle_netsim::Fault;
use rekindle_records::port::RouteClass;
use rekindle_types::dht_layout::member_record::{BEAT, CARD, ROUTES};
use rekindle_types::dto::community_profile::ProfileField;
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

use super::{community_key, wait_for_review, LIMIT};
use crate::harness::{eventually, Community};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn re_enrol_from_a_cloned_root_retires_the_original_key() {
    let mut community = Community::create(4, 7029).await;
    community.join(1).await;
    community.join(3).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.nodes[1].ok(IpcRequest::Lock).await;
    let backup = crate::data_root::snapshot(&community.sim, &community.nodes[1]);
    crate::devices::unlock(&community, 1).await;

    let role = crate::devices::self_assignable_role(&community, "after the copy").await;
    assert!(
        eventually(&community.sim, LIMIT, || crate::equivocation::shows(
            &community,
            1,
            "after the copy"
        ))
        .await
    );
    crate::devices::self_assign(&community, 1, &role)
        .await
        .unwrap();
    let member = community.pseudonym(1).await;
    assert!(crate::devices::sees_role(&community, 0, &member, &role).await);

    community.nodes[2].ok(IpcRequest::Lock).await;
    crate::data_root::restore(&community.sim, &community.nodes[2], &backup);
    let unused = community.nodes.remove(2);
    let mut copy = unused.restart(&community.sim, Duration::from_secs(1)).await;
    copy.public_key = community.nodes[1].public_key.clone();
    community.nodes.insert(2, copy);
    let author = PseudonymKey::from_hex_lossy(&member);
    let original_events = community.nodes[1].events().await;
    let old_record = assert_copy_catches_up_without_publishing(&community, &author).await;
    // The CLI command maps directly to this request. The harness applies
    // the requested proof through the same gate as the CLI.
    let result = community.nodes[2]
        .call_held(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityDeviceReEnrol {},
        )))
        .await
        .unwrap();
    assert!(result["reEnrolled"][0].get("error").is_none(), "{result:?}");
    assert!(
        eventually(&community.sim, LIMIT, || async {
            let original = community.nodes[1].host.governance();
            rekindle_governance_runtime::device_re_enrol::retired_from(&original, &community.anchor)
                .is_ok_and(|retired| retired.is_some())
        })
        .await,
        "the original merges the retirement carried on the new record"
    );
    let signing_stopped = |event: &SubscriptionEvent| {
        matches!(
            event,
            SubscriptionEvent::Notification(NotificationEvent::SystemAlert { title, .. }) if title == "Signing stopped"
        )
    };
    assert!(
        eventually(&community.sim, LIMIT, || async {
            original_events.any(signing_stopped)
        })
        .await,
        "the original receives its signing-stopped alert"
    );
    crate::equivocation::create_role(&community, 0, "after re-enrolment").await;
    assert!(
        eventually(&community.sim, LIMIT, || {
            crate::equivocation::shows(&community, 1, "after re-enrolment")
        })
        .await
    );
    assert_eq!(
        original_events.matching(signing_stopped),
        1,
        "one alert in the original session"
    );
    community.nodes[1].ok(IpcRequest::Lock).await;
    crate::devices::unlock(&community, 1).await;
    wait_for_review(&community, 1).await;
    re_admit_original_and_test_routes(&community, &author, &old_record).await;
}

async fn assert_copy_catches_up_without_publishing(
    community: &Community,
    author: &PseudonymKey,
) -> String {
    let view = community.nodes[3]
        .host
        .governance()
        .governance_state(&community.anchor)
        .unwrap();
    let old_record = view.members[author].member_record.clone();
    let writes =
        || [CARD, BEAT, ROUTES].map(|subkey| community.sim.app_sets_at(2, &old_record, subkey));
    let before_writes = writes();
    for node in [0, 3] {
        community.sim.fault(Fault::Unanswered {
            node,
            unanswered: true,
        });
    }
    crate::devices::unlock(community, 2).await;
    let copy = community.nodes[2].host.governance();
    let barrier = copy.unlock_barrier(&community.anchor);
    let old_original = community.nodes[1]
        .routes
        .own_blob(RouteClass::General)
        .unwrap();
    community.nodes[1]
        .routes
        .replace_own_class(RouteClass::General);
    community.nodes[2]
        .routes
        .replace_own_class(RouteClass::General);
    let original_route = || community.nodes[1].routes.own_blob(RouteClass::General);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            original_route().is_some_and(|route| {
                route != old_original
                    && network_route(community, author, &old_record) == Some(route)
            }) && copy
                .netsim_call_member_record(&community.anchor, &old_record, &[0])
                .await
                .is_ok()
        })
        .await,
        "the pending copy can call the original on its rotated, pinned record"
    );
    assert_eq!(barrier.phase(), BarrierPhase::CatchingUp);
    let _profile = community.nodes[2]
        .answer(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunitySetDisplayName {
                community: community.anchor.clone(),
                display_name: ProfileField::Set("Pending copy".into()),
            },
        )))
        .await;
    assert_eq!(
        writes(),
        before_writes,
        "the pending copy never writes its original's M/0, M/1 or M/6"
    );
    community.nodes[2].ok(IpcRequest::Lock).await;
    assert_eq!(
        writes(),
        before_writes,
        "the copy's quit does not write the original record"
    );

    restore_copy_only_through_dee(community).await;
    assert_eq!(
        writes(),
        before_writes,
        "the restored copy leaves the old record alone"
    );
    community.sim.fault(Fault::Unanswered {
        node: 0,
        unanswered: false,
    });
    old_record
}

async fn restore_copy_only_through_dee(community: &Community) {
    let view = community.nodes[3]
        .host
        .governance()
        .governance_state(&community.anchor)
        .unwrap();
    let dee = &community.nodes[3];
    let dee_author = community.pseudonym(3).await;
    let dee_record = view.members[&PseudonymKey::from_hex_lossy(&dee_author)]
        .member_record
        .clone();
    let old_dee = dee.routes.own_blob(RouteClass::General).unwrap();
    crate::devices::unlock(community, 2).await;
    let copy = community.nodes[2].host.governance();
    assert!(
        eventually(&community.sim, LIMIT, || async {
            copy.cached_member_route(&community.anchor, &dee_author) == Some(old_dee.clone())
        })
        .await,
        "the copy has Dee's old route before the watch hint is dropped"
    );
    community.sim.fault(Fault::DropValueChanges {
        node: 2,
        key: dee_record.clone(),
        drop: true,
    });
    community.sim.fault(Fault::OutsideDead {
        node: 3,
        class: RouteClass::General,
    });
    let old_seq = community.sim.network_get(&dee_record, ROUTES).unwrap().seq;
    dee.routes.replace_own_class(RouteClass::General);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            community
                .sim
                .network_get(&dee_record, ROUTES)
                .is_some_and(|value| value.seq > old_seq)
        })
        .await,
        "Dee's replacement is visible in M/6"
    );
    community.sim.fault(Fault::Unanswered {
        node: 3,
        unanswered: false,
    });
    wait_for_review(community, 2).await;
    assert!(
        matches!(
            copy.unlock_barrier(&community.anchor).phase(),
            BarrierPhase::Restored(_)
        ),
        "Dee is the only answering ring peer and gives the copy a restored verdict"
    );
}

async fn re_admit_original_and_test_routes(
    community: &Community,
    author: &PseudonymKey,
    old_record: &str,
) {
    let second = community.nodes[1]
        .call_held(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityDeviceReEnrol {},
        )))
        .await
        .unwrap();
    assert!(second["reEnrolled"][0].get("error").is_none(), "{second:?}");
    let second_key = community_key(community, 1);
    let records = || {
        let state = community.nodes[0]
            .host
            .governance()
            .governance_state(&community.anchor)
            .unwrap();
        rekindle_governance_runtime::member_card::member_device_records(
            &state,
            &community.anchor,
            author,
        )
        .into_iter()
        .map(|at| at.record.to_owned())
        .collect::<Vec<_>>()
    };
    assert!(
        eventually(&community.sim, LIMIT, || async {
            community.nodes[0]
                .host
                .governance()
                .governance_state(&community.anchor)
                .unwrap()
                .devices
                .member(author)
                .is_some_and(|devices| {
                    devices
                        .endorsements
                        .values()
                        .any(|e| e.device_key == second_key)
                })
        })
        .await,
        "the creator sees the original's fresh endorsement"
    );
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityDeviceReadmit {
                anchor: community.anchor.clone(),
                member: author.to_hex(),
                device_key: hex::encode(second_key),
            },
        )))
        .await;
    assert!(
        eventually(&community.sim, LIMIT, || async { records().len() == 2 }).await,
        "both fresh device bindings are effective after rule 7 readmission"
    );
    let records = records();
    assert!(records.iter().all(|record| record != old_record));
    assert_independent_routes(community, author).await;
}

async fn assert_independent_routes(community: &Community, author: &PseudonymKey) {
    let reader = community.nodes[0].host.governance();
    let mut expected = Vec::new();
    for node in [1, 2] {
        let state = community.nodes[node]
            .host
            .governance()
            .governance_state(&community.anchor)
            .unwrap();
        let key = community_key(community, node);
        let device = state
            .devices
            .member(author)
            .unwrap()
            .endorsements
            .values()
            .find(|e| e.device_key == key)
            .unwrap();
        let record = device.device_record.clone();
        let previous = community.nodes[node]
            .routes
            .own_blob(RouteClass::General)
            .unwrap();
        community.nodes[node]
            .routes
            .replace_own_class(RouteClass::General);
        assert!(
            eventually(&community.sim, LIMIT, || async {
                let Some(current) = community.nodes[node].routes.own_blob(RouteClass::General)
                else {
                    return false;
                };
                current != previous && network_route(community, author, &record) == Some(current)
            })
            .await,
            "the effective device rotates its own M/6"
        );
        assert!(reader
            .netsim_call_member_record(&community.anchor, &record, &[0])
            .await
            .is_ok());
        expected.push((
            record,
            community.nodes[node].routes.own_blob(RouteClass::General),
        ));
    }
    for (record, route) in expected {
        assert_eq!(network_route(community, author, &record), route);
    }
}

fn network_route(community: &Community, author: &PseudonymKey, record: &str) -> Option<Vec<u8>> {
    use rekindle_codec::member_record::{routes::decode_member_routes, RecordScope};
    let value = community.sim.network_get(record, ROUTES)?;
    let at = RecordScope {
        community_id: &community.anchor,
        member_record: record,
    };
    decode_member_routes(&value.data, at, &author.0)
        .ok()
        .map(|routes| routes.general)
}
