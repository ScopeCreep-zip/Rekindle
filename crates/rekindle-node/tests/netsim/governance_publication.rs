//! Our governance ops' publication state from the backend (review RV7 M2,
//! RV4 M3): `GovernanceEvent::OpPublication` is `Pending` while the op is
//! queued, announced again at every unlock while its row stays, and
//! `Landed` once it is on our member record and applied. An invite whose
//! command died mid-wait (as a crash kills it) is revoked by the row
//! stored with its `InviteCreated`, so it never stays live. Concurrent
//! joins of one community mint one `MemberJoined`.

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelRequest, LifecycleRequest};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_netsim::{Fault, Records, Writes};
use rekindle_types::dht_layout::member_record;
use rekindle_types::subscription_events::{GovernanceEvent, OpPublicationState, SubscriptionEvent};

use crate::harness::{eventually, Community, Events, PASSPHRASE};
use crate::invite_unpublished::creator_record;

const LIMIT: Duration = Duration::from_secs(240);

/// The distinct ops `events` announced in a state `want` accepts, in order.
fn published(events: &Events, want: &dyn Fn(&OpPublicationState) -> bool) -> Vec<String> {
    let ops = std::cell::RefCell::new(Vec::<String>::new());
    events.any(|e| {
        if let SubscriptionEvent::Governance(GovernanceEvent::OpPublication { op, state, .. }) = e {
            let mut ops = ops.borrow_mut();
            if want(state) && !ops.contains(op) {
                ops.push(op.clone());
            }
        }
        false
    });
    ops.into_inner()
}

/// Every write of the community's creation settles first.
async fn settle() {
    tokio::time::sleep(Duration::from_secs(60)).await;
}

fn channel_create(community: &Community) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::ChannelCreate {
        community: community.anchor.clone(),
        name: "pending".into(),
        kind: "text".into(),
        category: None,
        topic: None,
        slowmode_seconds: 0,
        parent_voice_channel: None,
    }))
}

/// A queued op is pending, pending again after a lock and unlock (its row
/// is re-announced at the outbox's start), and landed once the fault lifts.
/// Fails without the re-announcement: no pending event after the unlock.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_queued_op_is_pending_again_at_unlock_and_landed_once_it_lands() {
    let community = Community::create(1, 241).await;
    settle().await;
    let creator = &community.nodes[0];
    community.sim.fault(Fault::WriteFails {
        node: 0,
        of: Writes::Record(creator_record(&community)),
    });
    let before = creator.events().await;
    creator.ok(channel_create(&community)).await;
    let pending = published(&before, &|s| *s == OpPublicationState::Pending);
    assert_eq!(pending.len(), 1, "one op is pending: {pending:?}");

    let key = creator.public_key.clone().expect("an identity");
    creator.ok(IpcRequest::Lock).await;
    let after = creator.events().await;
    creator
        .ok(IpcRequest::Unlock {
            identity: key,
            passphrase: PASSPHRASE.into(),
        })
        .await;
    let again = eventually(&community.sim, LIMIT, || async {
        published(&after, &|s| *s == OpPublicationState::Pending).contains(&pending[0])
    })
    .await;
    assert!(again, "the queued op is announced pending at the unlock");

    community.sim.fault(Fault::Heal);
    let landed = eventually(&community.sim, LIMIT, || async {
        published(&after, &|s| *s == OpPublicationState::Landed).contains(&pending[0])
    })
    .await;
    assert!(landed, "the op is announced landed once it lands");
}

/// A crash between an invite's `InviteCreated` and its revoke never leaves
/// a live invite (review RV4 M3). The crash is the command dying mid-wait:
/// its future is dropped 20 s in, after its rows are stored and before any
/// answer, so the command's own tombstone write never runs. The two rows
/// are stored in one transaction, so once the fault lifts the
/// `InviteCreated` lands and the revoke row behind it revokes it. Fails
/// with the `InviteCreated` row stored alone: the invite lands live and
/// stays listed.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_invite_whose_command_died_mid_wait_is_revoked_once_it_lands() {
    let community = Community::create(1, 243).await;
    settle().await;
    let creator = &community.nodes[0];
    community.sim.fault(Fault::WriteFails {
        node: 0,
        of: Writes::Record(creator_record(&community)),
    });
    let events = creator.events().await;
    let create = creator.call(IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::InviteCreate {
            community: community.anchor.clone(),
            max_uses: 1,
            expires_seconds: None,
        },
    )));
    let died = tokio::time::timeout(Duration::from_secs(20), create).await;
    assert!(died.is_err(), "the command was still waiting: {died:?}");

    community.sim.fault(Fault::Heal);
    let list = || {
        IpcRequest::Community(CommunityRequest::Lifecycle(LifecycleRequest::InviteList {
            community: community.anchor.clone(),
        }))
    };
    // The `InviteCreated` lands, then the revoke written behind it.
    let revoked = eventually(&community.sim, LIMIT, || async {
        published(&events, &|s| *s == OpPublicationState::Landed).len() >= 2
            && creator
                .ok(list())
                .await
                .as_array()
                .is_some_and(Vec::is_empty)
    })
    .await;
    let listed = creator.ok(list()).await;
    let landed = published(&events, &|s| *s == OpPublicationState::Landed);
    assert!(
        revoked,
        "the landed invite is revoked: listed {listed}, landed {landed:?}"
    );
}

/// Two joins of one community at once mint one `MemberJoined`: the second
/// waits on the join lock and answers the first's membership (review RV7
/// M3). Fails without the lock: two distinct MemberJoined ops are pending.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn concurrent_joins_mint_one_member_joined() {
    let mut community = Community::create(2, 247).await;
    settle().await;
    let link = community.invite().await;
    community.sim.fault(Fault::WriteFails {
        node: 1,
        of: Writes::Subkey {
            subkey: member_record::GOV_HEAD,
            record: Some(Records::CreatedDflt(member_record::SUBKEY_COUNT)),
        },
    });
    let joiner = &community.nodes[1];
    let events = joiner.events().await;
    let join = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityJoin { invite: link },
    ));
    let (first, second) = tokio::join!(joiner.call(join.clone()), joiner.call(join));
    let (first, second) = (first.expect("joined"), second.expect("joined"));
    assert_eq!(
        (&first["slot"], &first["segment"]),
        (&second["slot"], &second["segment"]),
        "both answer one membership: {first} / {second}"
    );
    let pending = published(&events, &|s| *s == OpPublicationState::Pending);
    assert_eq!(pending.len(), 1, "one MemberJoined is minted: {pending:?}");
}
