//! A join the inviter refuses after the join returned (plan E3.1/JN1): the
//! inviter did not answer the join's bootstrap exchange, so the join stood
//! from the DHT and the exchange became a `bootstrap` outbox row
//! (`host/adapters/outbox/bootstrap.rs`). When the inviter answers the
//! retry, it refuses the second redemption of a single-use invite
//! (`merge/gates.rs` `invite_gate`), and the joiner withdraws the join
//! (`join_bootstrap.rs` `withdraw_refused_join`). Every frontend must hear
//! why the community it held is gone: exactly one
//! `MembershipEvent::JoinWithdrawn` with the inviter's reason and the
//! Host's words for it, never a silent disappearance; a join that stands
//! publishes none.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_netsim::{Fault, Writes};
use rekindle_types::join_refusal::JoinRefusal;
use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, Events};

/// The `JoinWithdrawn` events about `anchor` that `events` holds.
fn withdrawn(events: &Events, anchor: &str) -> usize {
    events.matching(|e| {
        matches!(e, SubscriptionEvent::Membership(MembershipEvent::JoinWithdrawn {
            community, ..
        }) if community == anchor)
    })
}

/// Whether node `i` still holds the community.
async fn holds(community: &Community, i: usize) -> bool {
    community.nodes[i]
        .call(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityInfo {
                anchor: community.anchor.clone(),
            },
        )))
        .await
        .is_ok()
}

/// Node 1 joined and the inviter (node 0) merged it, taking the single
/// use; node 2 then joined while the inviter answered nothing, so its join
/// stands from the DHT and its exchange is a queued `bootstrap` row. Each
/// joiner's events from before its join.
async fn second_join_queued(seed: u64) -> (Community, Events, Events) {
    let mut community = Community::create(3, seed).await;
    let link = community.invite().await;
    // The inviter's frontier never cites the first join, so the second
    // joiner's own check finds the single use still free.
    community.sim.fault(Fault::WriteFails {
        node: 0,
        of: Writes::Record(crate::invite_unpublished::creator_record(&community)),
    });
    let first_seen = community.nodes[1].events().await;
    let second_seen = community.nodes[2].events().await;
    community.join_with(1, link.clone()).await;
    // The inviter has merged the first join: the one use is taken.
    assert!(
        eventually(&community.sim, Duration::from_secs(600), || async {
            community.info(0).await["member_count"].as_u64() == Some(2)
        })
        .await,
        "the inviter merged the first join"
    );
    // The inviter answers nothing: the second join stands from the DHT and
    // returns, its exchange queued.
    community.sim.fault(Fault::Unanswered {
        node: 0,
        unanswered: true,
    });
    let second = community.nodes[2]
        .call(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityJoin { invite: link },
        )))
        .await;
    assert!(
        second.is_ok(),
        "the join returns while the inviter answers nothing: {second:?}"
    );
    assert!(holds(&community, 2).await, "the returned join is held");
    assert_eq!(
        withdrawn(&second_seen, &community.anchor),
        0,
        "nothing is withdrawn before the inviter answers"
    );
    (community, first_seen, second_seen)
}

/// Exactly one `JoinWithdrawn` for node 2, with the inviter's reason and
/// the Host's words; none for node 1, whose join stands.
async fn withdrawn_once(community: &Community, first_seen: &Events, second_seen: &Events) {
    assert!(
        eventually(&community.sim, Duration::from_secs(600), || async {
            withdrawn(second_seen, &community.anchor) > 0
        })
        .await,
        "the joiner hears its join was withdrawn"
    );
    assert!(
        !holds(community, 2).await,
        "the withdrawn join is forgotten"
    );
    tokio::time::sleep(rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD * 2).await;
    let anchor = community.anchor.clone();
    assert_eq!(
        withdrawn(second_seen, &anchor),
        1,
        "exactly one JoinWithdrawn"
    );
    let reason = JoinRefusal::InviteUsedUp;
    assert!(
        second_seen.any(|e| matches!(e,
            SubscriptionEvent::Membership(MembershipEvent::JoinWithdrawn {
                community, reason: r, reason_text,
            }) if *community == anchor && *r == reason && *reason_text == reason.withdrawn_text())),
        "the JoinWithdrawn carries the inviter's reason and the Host's words"
    );
    assert_eq!(
        withdrawn(first_seen, &anchor),
        0,
        "a join that stands publishes no JoinWithdrawn"
    );
    assert!(holds(community, 1).await, "the first join stands");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_join_refused_after_it_returned_is_withdrawn_with_its_reason() {
    let (community, first_seen, second_seen) = second_join_queued(4414).await;
    // The inviter answers the retried exchange and refuses it.
    community.sim.fault(Fault::Unanswered {
        node: 0,
        unanswered: false,
    });
    withdrawn_once(&community, &first_seen, &second_seen).await;
}

/// Finding JW2 (real app): the joiner locked while its exchange was
/// queued, the inviter came back, and the joiner unlocked. Its outbox
/// runs from the unlock's first step, before the communities are restored:
/// the queued row read an empty cache as a community left and was dropped,
/// so the retried exchange never left and the refusal never came. The row
/// survives the lock, is re-driven once the communities are restored, the
/// inviter refuses it, and the join is withdrawn once.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_join_queued_across_a_lock_is_retried_at_unlock_and_withdrawn_once() {
    let (community, first_seen, second_seen) = second_join_queued(4415).await;
    let joiner = &community.nodes[2];
    let key = joiner.public_key.clone().expect("an identity");
    joiner.ok(IpcRequest::Lock).await;
    // The inviter comes back while the joiner is away; the row's backoff
    // runs out meanwhile.
    community.sim.fault(Fault::Unanswered {
        node: 0,
        unanswered: false,
    });
    tokio::time::sleep(Duration::from_secs(180)).await;
    assert_eq!(
        withdrawn(&second_seen, &community.anchor),
        0,
        "a locked joiner withdraws nothing"
    );
    joiner
        .ok(IpcRequest::Unlock {
            identity: key,
            passphrase: crate::harness::PASSPHRASE.into(),
        })
        .await;
    withdrawn_once(&community, &first_seen, &second_seen).await;
}
