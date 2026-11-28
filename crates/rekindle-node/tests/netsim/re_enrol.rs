//! Host re-enrolment reviews on a restored data root and on a live copy.

use std::time::Duration;

use rekindle_governance_runtime::unlock_barrier::BarrierPhase;
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, IpcResponse, LifecycleRequest};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::member_record::GOV_HEAD;
use rekindle_types::intent::{HeldReview, IntentKind, IntentProof};
use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

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

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn re_enrol_barrier_and_own_key_retired_share_one_review() {
    let (community, backup) = solo_creator_and_restored_copy(7032).await;
    let copy_review = wait_for_review(&community, 1).await;
    let original = &community.nodes[0];
    original.ok(IpcRequest::Lock).await;
    crate::data_root::restore(&community.sim, original, &backup);
    crate::devices::unlock(&community, 0).await;

    let first = wait_for_review(&community, 0).await;
    let governance = original.host.governance();
    assert!(matches!(
        governance.unlock_barrier(&community.anchor).phase(),
        BarrierPhase::Restored(_)
    ));

    wait_for_own_record_watch(&community).await;
    let events = original.events().await;
    // The copy's commit writes a retirement pointer to the original's
    // watched record. The original's earlier barrier verdict still gives
    // both producers the same review facts.
    let answer = community.nodes[1]
        .ok(IpcRequest::IntentCommit {
            intent_id: copy_review.review.intent_id,
            proof: IntentProof::Acknowledged,
        })
        .await;
    assert!(answer["reEnrolled"][0].get("error").is_none(), "{answer:?}");
    assert!(
        eventually(&community.sim, LIMIT, || async {
            events.any(|event| {
                matches!(
                    event,
                    SubscriptionEvent::Notification(NotificationEvent::SystemAlert { title, .. })
                        if title == "Signing stopped"
                )
            })
        })
        .await,
        "the original's own-record watch reported its key's retirement"
    );
    tokio::time::sleep(Duration::from_secs(1)).await;

    let SubscriptionEvent::HeldReviews(held) = original.host.held_reviews() else {
        panic!("expected the held set");
    };
    let held: Vec<_> = held
        .into_iter()
        .filter(|item| item.review.kind == IntentKind::ReEnrolDevice)
        .collect();
    assert_eq!(held, vec![first.clone()], "both sources keep the same hold");
    assert!(!events.any(|event| matches!(
        event,
        SubscriptionEvent::IntentPending { review, .. } if review.kind == IntentKind::ReEnrolDevice
    )));
    original
        .ok(IpcRequest::IntentCancel {
            intent_id: first.review.intent_id,
        })
        .await;
    assert!(review(&community, 0).is_none());
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
    use rekindle_governance_runtime::member_card::{member_device_records, read_card};
    let governance = community.nodes[0].host.governance();
    let p = governance.my_pseudonym(anchor).unwrap();
    let merged = governance.governance_state(anchor).unwrap();
    let bindings = member_device_records(&merged, anchor, &p);
    assert_eq!(
        bindings.len(),
        1,
        "one effective record remains for the member"
    );
    let pool = governance.record_port().unwrap();
    read_card(&*pool, &bindings[0])
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
    assert!(
        eventually(&community.sim, LIMIT, || async {
            [first.as_str(), second]
                .into_iter()
                .enumerate()
                .all(|(i, anchor)| {
                    device_key(&community, 0, anchor) != old_keys[i]
                        && community.nodes[0]
                            .host
                            .governance()
                            .unlock_barrier(anchor)
                            .view()
                            .is_none()
                })
        })
        .await,
        "both communities complete their post-unlock re-enrolment"
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

#[path = "re_enrol_cloned_root.rs"]
mod cloned_root;

async fn solo_creator_and_restored_copy(seed: u64) -> (Community, crate::data_root::Backup) {
    let mut community = Community::create(2, seed).await;
    community.nodes[0].ok(IpcRequest::Lock).await;
    let backup = crate::data_root::snapshot(&community.sim, &community.nodes[0]);
    crate::devices::unlock(&community, 0).await;
    crate::equivocation::create_role(&community, 0, "after the copy").await;

    community.nodes[1].ok(IpcRequest::Lock).await;
    crate::data_root::restore(&community.sim, &community.nodes[1], &backup);
    let unused = community.nodes.remove(1);
    let mut copy = unused.restart(&community.sim, Duration::from_secs(1)).await;
    copy.public_key.clone_from(&community.nodes[0].public_key);
    community.nodes.push(copy);
    crate::devices::unlock(&community, 1).await;
    (community, backup)
}

async fn wait_for_own_record_watch(community: &Community) -> String {
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
                .watched_subkeys(0, &old_record)
                .contains(&GOV_HEAD)
        })
        .await,
        "the original's own-record watch is armed"
    );
    old_record
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_solo_members_original_device_learns_its_re_enrolment_from_its_own_record() {
    let (community, _backup) = solo_creator_and_restored_copy(7031).await;
    let original_events = community.nodes[0].events().await;
    let held = wait_for_review(&community, 1).await;
    let original = community.nodes[0].host.governance();
    let old_record = wait_for_own_record_watch(&community).await;
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
    // The view agrees with the review: the retired original must re-enrol.
    assert!(
        crate::devices::device_status(&community, 0, "needsReenrol").await,
        "the original's device view reads needsReenrol once its key is retired"
    );
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

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn re_enrol_recovers_a_carrier_after_cutover_before_the_outbox() {
    let (mut community, _backup) = solo_creator_and_restored_copy(7033).await;
    let held = wait_for_review(&community, 1).await;
    let old_record = wait_for_own_record_watch(&community).await;
    let original_head = community.sim.network_get(&old_record, GOV_HEAD).unwrap();
    let old_key = community_key(&community, 1);
    let db = community.nodes[1].db.current().expect("unlocked database");
    // Stop immediately after the device-vault transaction. A database
    // failure at this seam leaves no signed-op or outbox row to replay.
    db.call(|conn| {
        conn.execute_batch(
            "CREATE TEMP TRIGGER interrupt_carrier BEFORE INSERT ON governance_ops
         WHEN NEW.signed_here = 1 AND NEW.device_seq = 0
         BEGIN SELECT RAISE(FAIL, 'interrupted retirement carrier'); END;",
        )
    })
    .await
    .unwrap();
    let answer = community.nodes[1]
        .ok(IpcRequest::IntentCommit {
            intent_id: held.review.intent_id,
            proof: IntentProof::Acknowledged,
        })
        .await;
    assert!(
        answer["reEnrolled"][0]["error"]
            .as_str()
            .is_some_and(|error| error.contains("interrupted retirement carrier")),
        "{answer:?}"
    );
    let successor = community_key(&community, 1);
    assert_ne!(old_key, successor);
    assert_eq!(
        community
            .sim
            .network_get(&old_record, GOV_HEAD)
            .unwrap()
            .seq,
        original_head.seq
    );
    db.call(|conn| conn.execute_batch("DROP TRIGGER interrupt_carrier"))
        .await
        .unwrap();
    drop(db);

    let stopped = community.nodes.remove(1);
    let restarted = stopped
        .restart(&community.sim, Duration::from_secs(1))
        .await;
    community.nodes.insert(1, restarted);
    crate::devices::unlock(&community, 1).await;
    assert_eq!(
        community_key(&community, 1),
        successor,
        "the vault kept the same binding"
    );
    wait_for_review(&community, 0).await;
    let port = community.nodes[0].host.governance().record_port().unwrap();
    let (_, pointer) = rekindle_records::chain::read_log_with_retirement(&*port, &old_record, 0)
        .await
        .unwrap();
    assert_eq!(
        pointer
            .expect("the resumed carrier published its locator")
            .successor_key,
        successor
    );
}
