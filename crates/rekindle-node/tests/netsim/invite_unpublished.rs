//! An invite link is handed out only after its `InviteCreated` op and our
//! frontier are on our member record `M` (plan E3.2 S6 (4)): a joiner's
//! walk reads what is on the DHT. While every write of the inviter's `M`
//! lands below consensus (`Fault::WriteFails`, as a Veilid `set_value`
//! can), the invite's secrets record still lands but its op does not, so
//! invite create waits through the outbox's retries up to the command's
//! deadline and is refused "not yet published", with no link. Once the
//! fault clears, a new invite lands and a joiner uses it. The refused
//! invite's `InviteRevoked` is queued behind its `InviteCreated` on the same
//! stream, so it lands revoked and is never listed (plan E3.2 S6b).
//!
//! Every other governance command answers once its op's first attempt is
//! known or `GOVERNANCE_ANSWER_BOUND` passed, a success either way (RV4
//! review M1/M2, Matrix's local echo): a channel created while its op
//! cannot land answers Ok, pending, and the op, still queued, reaches the
//! other members once the fault clears.

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelRequest, LifecycleRequest};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_netsim::{Fault, Records, Writes};
use rekindle_types::dht_layout::{community_anchor, member_record};

use crate::harness::{eventually, Community};

/// The creator's member record `M`, where its op log lives.
pub(crate) fn creator_record(community: &Community) -> String {
    let cert = community
        .sim
        .network_get(&community.governance_key, community_anchor::GENESIS_CERT)
        .expect("the anchor's GenesisCert");
    rekindle_codec::governance_op::genesis::decode_genesis(&cert.data)
        .expect("a GenesisCert")
        .signed
        .cert
        .creator_member_record
}

fn invite_list(community: &Community) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Lifecycle(LifecycleRequest::InviteList {
        community: community.governance_key.clone(),
    }))
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn no_invite_link_until_its_op_is_on_the_dht() {
    let mut community = Community::create(2, 223).await;
    let record = creator_record(&community);
    community.sim.fault(Fault::WriteFails {
        node: 0,
        of: Writes::Record(record),
    });
    let refused = community.nodes[0]
        .call(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::InviteCreate {
                community: community.governance_key.clone(),
                max_uses: 1,
                expires_seconds: None,
            },
        )))
        .await;
    let answer = format!("{refused:?}");
    assert!(
        refused.is_err() && answer.contains("not yet published"),
        "no link while the op is off the DHT: {answer}"
    );

    let listed = community.nodes[0].ok(invite_list(&community)).await;
    assert_eq!(
        listed.as_array().map(Vec::len),
        Some(0),
        "the refused invite is revoked behind its op: {listed}"
    );

    community.sim.fault(Fault::Heal);
    // A fresh invite (its op lands behind the queued ones) and a join by it.
    community.join(1).await;
    // The joiner walked both queued ops from the DHT: the refused invite
    // reached it revoked, so it lists what the inviter lists.
    let ids = |listed: serde_json::Value| -> Vec<String> {
        listed.as_array().map_or_else(Vec::new, |all| {
            all.iter()
                .filter_map(|i| i["inviteId"].as_str().map(str::to_owned))
                .collect()
        })
    };
    let ours = ids(community.nodes[0].ok(invite_list(&community)).await);
    let theirs = ids(community.nodes[1].ok(invite_list(&community)).await);
    assert_eq!(ours.len(), 1, "only the join's invite: {ours:?}");
    assert_eq!(
        ours, theirs,
        "the refused invite is revoked for the joiner too"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_governance_command_answers_pending_within_the_bound() {
    let mut community = Community::create(2, 227).await;
    community.join(1).await;
    let record = creator_record(&community);
    community.sim.fault(Fault::WriteFails {
        node: 0,
        of: Writes::Record(record),
    });
    let create = IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::ChannelCreate {
        community: community.governance_key.clone(),
        name: "queued".into(),
        kind: "text".into(),
        category: None,
        topic: None,
        slowmode_seconds: 0,
        parent_voice_channel: None,
    }));
    let asked = tokio::time::Instant::now();
    let answer = community.nodes[0].call(create).await;
    let waited = asked.elapsed();
    assert!(answer.is_ok(), "a pending op is a success: {answer:?}");
    // The channel's own record is created before its op; the op's wait is
    // the bound, never the command's deadline (about 168 s).
    assert!(
        waited <= rekindle_types::config::GOVERNANCE_ANSWER_BOUND * 2,
        "answered within the bound, not the command's deadline: {waited:?}"
    );

    community.sim.fault(Fault::Heal);
    let list = IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::ChannelList {
        community: community.governance_key.clone(),
    }));
    let landed = eventually(&community.sim, Duration::from_secs(900), || async {
        community.nodes[1]
            .call(list.clone())
            .await
            .is_ok_and(|v| v.to_string().contains("\"queued\""))
    })
    .await;
    let ours = community.nodes[0].ok(list.clone()).await;
    let theirs = community.nodes[1].ok(list).await;
    assert!(
        landed,
        "the queued op reaches the other member: {ours} / {theirs}"
    );
}

/// A join whose `MemberJoined` cannot land (every set node 1 makes of the
/// op log head, `M`/2, of a `DFLT(8)` record it created, its own member
/// record, is refused, so its card lands and its op does not, while its
/// other records' writes are untouched)
/// answers Ok, pending, with its local side done and the op queued (plan
/// E3.2 S6b, RV4 review M1). The join is idempotent: asked again, it
/// answers the same membership and mints no second record. Once the fault
/// lifts the op lands: a later op of the joiner, behind it on the same
/// stream, lands.
///
/// Revert run (review RV7 L2, 2026-11-14): with the guard off
/// (`already_joined` never answering) the repeated join runs the stages
/// again and is refused (a second member record's set lands below
/// consensus), so it fails before the count of minted ops is reached.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_join_whose_member_joined_cannot_land_is_pending_and_idempotent() {
    let mut community = Community::create(2, 229).await;
    community.sim.fault(Fault::WriteFails {
        node: 1,
        of: Writes::Subkey {
            subkey: member_record::GOV_HEAD,
            record: Some(Records::CreatedDflt(member_record::SUBKEY_COUNT)),
        },
    });
    let link = community.invite().await;
    let join = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityJoin { invite: link },
    ));
    let events = community.nodes[1].events().await;
    let joined = community.nodes[1].ok(join.clone()).await;
    assert_eq!(joined["pending"], true, "the join is pending: {joined}");
    let again = community.nodes[1].ok(join).await;
    assert_eq!(
        (&again["slot"], &again["segment"], &again["pending"]),
        (&joined["slot"], &joined["segment"], &joined["pending"]),
        "a repeated join answers the same membership: {joined} / {again}"
    );
    // Each run of the join stages mints one member record and its one
    // `MemberJoined` (RV7 review L2): the repeat minted neither.
    let minted = std::cell::RefCell::new(Vec::<String>::new());
    events.any(|e| {
        if let rekindle_types::subscription_events::SubscriptionEvent::Governance(
            rekindle_types::subscription_events::GovernanceEvent::OpPublication { op, .. },
        ) = e
        {
            let mut minted = minted.borrow_mut();
            if !minted.contains(op) {
                minted.push(op.clone());
            }
        }
        false
    });
    let minted = minted.into_inner();
    assert_eq!(
        minted.len(),
        1,
        "one MemberJoined and one member record: {minted:?}"
    );
    let listed = community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityList,
        )))
        .await;
    assert!(
        listed.to_string().contains(&community.governance_key),
        "the join's local side is done: {listed}"
    );

    community.sim.fault(Fault::Heal);
    let leave = community.nodes[1]
        .call(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityLeave {
                governance_key: community.governance_key.clone(),
            },
        )))
        .await;
    assert!(
        leave.is_ok(),
        "the queued MemberJoined lands, then the MemberLeft behind it: {leave:?}"
    );
}
