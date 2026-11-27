//! Host re-enrolment reviews on a restored data root and on a live copy.

use std::time::Duration;

use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, IpcResponse, LifecycleRequest};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::intent::{HeldReview, IntentKind, IntentProof};
use rekindle_types::subscription_events::SubscriptionEvent;

use crate::harness::{eventually, Community};

const LIMIT: Duration = Duration::from_secs(180);

fn review(community: &Community, node: usize) -> Option<HeldReview> {
    let SubscriptionEvent::HeldReviews(reviews) = community.nodes[node].host.held_reviews() else {
        unreachable!("the Host returns its held set");
    };
    reviews
        .into_iter()
        .find(|item| item.review.kind == IntentKind::ReEnrolDevice)
}

fn community_key(community: &Community, node: usize) -> [u8; 32] {
    device_key(community, node, &community.anchor)
}

fn device_key(community: &Community, node: usize, anchor: &str) -> [u8; 32] {
    let secret = community.nodes[node]
        .host
        .governance()
        .device_op_key(anchor)
        .unwrap()
        .expect("the held device key");
    SigningKey::from_bytes(&secret).verifying_key().to_bytes()
}

async fn wait_for_review(community: &Community, node: usize) -> HeldReview {
    assert!(
        eventually(&community.sim, LIMIT, || async {
            review(community, node).is_some()
        })
        .await,
        "Host raises its re-enrolment review"
    );
    review(community, node).unwrap()
}

async fn restore_creator(seed: u64) -> Community {
    let community = Community::of(3, seed).await;
    rewind_creator(&community).await;
    community
}

async fn rewind_creator(community: &Community) {
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.nodes[0].ok(IpcRequest::Lock).await;
    let backup = crate::data_root::snapshot(&community.sim, &community.nodes[0]);
    crate::devices::unlock(community, 0).await;
    crate::equivocation::create_role(community, 0, "after backup").await;
    assert!(
        eventually(&community.sim, LIMIT, || async {
            crate::equivocation::shows(community, 1, "after backup").await
        })
        .await
    );
    community.nodes[0].ok(IpcRequest::Lock).await;
    crate::data_root::restore(&community.sim, &community.nodes[0], &backup);
    crate::devices::unlock(community, 0).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn re_enrol_device_is_re_raised_at_every_unlock_until_committed() {
    let community = restore_creator(7026).await;
    let original_key = community_key(&community, 0);
    let first = wait_for_review(&community, 0).await;
    community.nodes[0]
        .ok(IpcRequest::IntentCancel {
            intent_id: first.review.intent_id,
        })
        .await;
    assert!(review(&community, 0).is_none());
    assert_eq!(community_key(&community, 0), original_key);

    community.nodes[0].ok(IpcRequest::Lock).await;
    crate::devices::unlock(&community, 0).await;
    let next = wait_for_review(&community, 0).await;
    assert_ne!(next.hold_id, first.hold_id);

    let answer = community.nodes[0]
        .ok(IpcRequest::IntentCommit {
            intent_id: next.review.intent_id,
            proof: IntentProof::Acknowledged,
        })
        .await;
    assert!(answer["reEnrolled"][0].get("error").is_none(), "{answer:?}");
    assert_ne!(community_key(&community, 0), original_key);

    community.nodes[0].ok(IpcRequest::Lock).await;
    crate::devices::unlock(&community, 0).await;
    tokio::time::sleep(LIMIT).await;
    assert!(
        review(&community, 0).is_none(),
        "the fresh key needs no re-enrolment"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn re_enrolment_waits_for_the_users_commit() {
    let community = restore_creator(7027).await;
    let held = wait_for_review(&community, 0).await;
    let original_key = community_key(&community, 0);
    let request = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityDeviceReEnrol {},
    ));
    let IpcResponse::IntentRequired(manual) = community.nodes[0].answer(request).await else {
        panic!("the CLI request must ask the user");
    };
    assert_eq!(manual.kind, IntentKind::ReEnrolDevice);
    let refused = community.nodes[0]
        .answer(IpcRequest::IntentCommit {
            intent_id: manual.intent_id,
            proof: IntentProof::Commit,
        })
        .await;
    assert!(matches!(refused, IpcResponse::Error { .. }), "{refused:?}");
    assert_eq!(community_key(&community, 0), original_key);

    let answer = community.nodes[0]
        .ok(IpcRequest::IntentCommit {
            intent_id: held.review.intent_id,
            proof: IntentProof::Acknowledged,
        })
        .await;
    assert!(answer["reEnrolled"][0].get("error").is_none(), "{answer:?}");
    assert_ne!(community_key(&community, 0), original_key);
}

async fn card_key(community: &Community, anchor: &str) -> [u8; 32] {
    use rekindle_governance_runtime::member_card::{read_card, MemberRecordRef};
    let governance = community.nodes[0].host.governance();
    let p = governance.my_pseudonym(anchor).unwrap();
    let merged = governance.governance_state(anchor).unwrap();
    let member = &merged.members[&p];
    let pool = governance.record_port().unwrap();
    read_card(
        pool.as_ref(),
        &MemberRecordRef {
            community_id: anchor,
            pseudonym: &p,
            record: &member.member_record,
            owner: &member.member_record_owner,
        },
    )
    .await
    .unwrap()
    .unwrap()
    .card
    .hpke_key
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn re_enrolment_replaces_every_per_device_secret_in_every_community() {
    let community = Community::of(3, 7028).await;
    let (sender, contact) = (&community.nodes[0], &community.nodes[1]);
    let sender_key = crate::pair::key(sender);
    let contact_key = crate::pair::key(contact);
    crate::pair::request(&community.sim, sender, contact).await;
    crate::pair::accept(contact, sender).await;
    assert!(
        eventually(&community.sim, LIMIT, || crate::pair::is_accepted_friend(
            sender,
            &contact_key
        ))
        .await
    );
    crate::pair::send(sender, &contact_key, "before re-enrolment").await;
    assert!(
        eventually(&community.sim, LIMIT, || crate::pair::holds_message(
            contact,
            &sender_key,
            "before re-enrolment"
        ))
        .await
    );
    let contact_events = contact.events().await;
    let second = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityCreate {
                name: "Second community".into(),
                description: String::new(),
                approval_required: false,
                co_creators: Vec::new(),
            },
        )))
        .await;
    let second = second["anchor"].as_str().unwrap();
    let first = &community.anchor;

    let old_keys = [
        device_key(&community, 0, first),
        device_key(&community, 0, second),
    ];
    let old_cards = [
        card_key(&community, first).await,
        card_key(&community, second).await,
    ];
    let prekeys = community.nodes[0].ok(IpcRequest::PrekeyStatus).await;
    rewind_creator(&community).await;
    let hold = wait_for_review(&community, 0).await;
    let reply = community.nodes[0]
        .ok(IpcRequest::IntentCommit {
            intent_id: hold.review.intent_id,
            proof: IntentProof::Acknowledged,
        })
        .await;
    let results = reply["reEnrolled"].as_array().unwrap();
    assert_eq!(results.len(), 2, "{reply:?}");
    assert!(
        results.iter().all(|row| row.get("error").is_none()),
        "{reply:?}"
    );

    let fresh = community.nodes[0].ok(IpcRequest::PrekeyStatus).await;
    for field in ["deviceId", "recordKey", "spkId", "pqLrId"] {
        assert!(
            !fresh[field].is_null(),
            "{field} missing after re-enrolment"
        );
        assert_ne!(fresh[field], prekeys[field], "{field} did not change");
    }
    for (i, anchor) in [first.as_str(), second].into_iter().enumerate() {
        assert_ne!(device_key(&community, 0, anchor), old_keys[i]);
        assert_ne!(card_key(&community, anchor).await, old_cards[i]);
    }
    assert!(eventually(&community.sim, LIMIT, || async {
        contact_events.any(|event| matches!(event, SubscriptionEvent::IntentPending { review, .. } if review.kind == IntentKind::AcceptSessionReset))
    }).await, "the contact is prompted to replace the session");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn re_enrol_from_a_cloned_root_retires_the_original_key() {
    use rekindle_types::subscription_events::NotificationEvent;
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
    crate::devices::unlock(&community, 2).await;
    let original_events = community.nodes[1].events().await;
    wait_for_review(&community, 2).await;
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
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_solo_members_original_device_learns_its_re_enrolment_from_its_own_record() {
    use rekindle_types::subscription_events::NotificationEvent;

    let mut community = Community::create(2, 7031).await;
    community.nodes[0].ok(IpcRequest::Lock).await;
    let backup = crate::data_root::snapshot(&community.sim, &community.nodes[0]);
    crate::devices::unlock(&community, 0).await;
    crate::equivocation::create_role(&community, 0, "after the copy").await;

    community.nodes[1].ok(IpcRequest::Lock).await;
    crate::data_root::restore(&community.sim, &community.nodes[1], &backup);
    let unused = community.nodes.remove(1);
    let mut copy = unused.restart(&community.sim, Duration::from_secs(1)).await;
    copy.public_key = community.nodes[0].public_key.clone();
    community.nodes.push(copy);
    crate::devices::unlock(&community, 1).await;

    let original_events = community.nodes[0].events().await;
    let held = wait_for_review(&community, 1).await;
    let original = community.nodes[0].host.governance();
    let state = original.governance_state(&community.anchor).unwrap();
    let old_record = rekindle_governance_runtime::device_signer::own_op_record(
        &original,
        &community.anchor,
        &state,
    )
    .unwrap();
    assert!(
        eventually(&community.sim, LIMIT, || async {
            community
                .sim
                .watchers_of_dflt(rekindle_types::dht_layout::member_record::SUBKEY_COUNT)
                .get(&old_record)
                .is_some_and(|watchers| watchers.contains(&0))
        })
        .await,
        "the original's own-record watch is armed"
    );
    for node in &community.nodes {
        let gov = node.host.governance();
        assert_eq!(
            gov.governance_state(&community.anchor)
                .unwrap()
                .members
                .len(),
            1
        );
        assert!(gov.mesh_peers(&community.anchor).is_empty());
    }
    assert!(review(&community, 0).is_none());

    let answer = community.nodes[1]
        .ok(IpcRequest::IntentCommit {
            intent_id: held.review.intent_id,
            proof: IntentProof::Acknowledged,
        })
        .await;
    assert!(answer["reEnrolled"][0].get("error").is_none(), "{answer:?}");
    wait_for_review(&community, 0).await;
    let port = original.record_port().unwrap();
    let (old_log, pointer) =
        rekindle_records::chain::read_log_with_retirement(&*port, &old_record, 0)
            .await
            .unwrap();
    assert!(!old_log.entries.is_empty(), "the old log stays reachable");
    let pointer = pointer.expect("the old GOV_HEAD has a retirement pointer");
    assert_eq!(pointer.successor_key, community_key(&community, 1));
    assert_ne!(pointer.successor_record, old_record);
    assert!(original_events.any(|event| matches!(
        event,
        SubscriptionEvent::Notification(NotificationEvent::SystemAlert { title, .. })
            if title == "Signing stopped"
    )));
    let state = original.governance_state(&community.anchor).unwrap();
    let (author, _) =
        rekindle_governance_runtime::op_log::own_pseudonym(&original, &community.anchor).unwrap();
    let device = rekindle_governance_runtime::device_signer::device_op_key(
        &original,
        &community.anchor,
        false,
    )
    .unwrap();
    assert!(matches!(
        rekindle_governance_runtime::device_signer::OpSigner::checked(
            &community.anchor,
            &state,
            author,
            &device,
        ),
        Err(rekindle_governance_runtime::GovernanceRuntimeError::OwnKeyRetired { .. })
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn re_enrol_reopens_the_old_prekey_owner_if_cutover_stops_before_commit() {
    let mut community = restore_creator(7030).await;
    let first = wait_for_review(&community, 0).await;
    let before = community.nodes[0].ok(IpcRequest::PrekeyStatus).await;
    let old_record = before["recordKey"].as_str().expect("our prekey record");
    community.nodes[0].host.netsim_stage_prekey_cutover();
    assert_eq!(
        community.nodes[0].host.netsim_prekey_cutover_state(),
        (old_record.to_owned(), true)
    );
    community.nodes[0].ok(IpcRequest::Lock).await;
    let stopped = community.nodes.remove(0);
    let restarted = stopped
        .restart(&community.sim, Duration::from_secs(1))
        .await;
    community.nodes.insert(0, restarted);
    crate::devices::unlock(&community, 0).await;
    let held = wait_for_review(&community, 0).await;
    assert_ne!(first.hold_id, held.hold_id);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            community.nodes[0].host.netsim_prekey_cutover_state() == (old_record.to_owned(), false)
        })
        .await,
        "the previous owner remains live and the staged owner is discarded"
    );
    let after = community.nodes[0].ok(IpcRequest::PrekeyStatus).await;
    assert_eq!(after["recordKey"], before["recordKey"]);
    assert_eq!(after["deviceId"], before["deviceId"]);
}
