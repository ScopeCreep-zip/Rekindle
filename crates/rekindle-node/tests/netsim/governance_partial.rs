//! A governance op whose write only some replicas take (RV4 review L3,
//! `Fault::WritePartial`): Veilid's `set_value` can end below consensus with
//! the replicas it counted holding the value (Veilid 0.5.8
//! `set_value.rs:711-738`, `storage_manager/mod.rs:990-1030`), while the writer, with
//! `AllowOffline(false)`, stores and queues nothing and hears `TryAgain`
//! (`local_record_store_interface.rs:60-63`). So an op the writer answers
//! pending may already be readable by a member whose fanout reaches those
//! replicas (never infer that a not-yet-published op is invisible, review
//! L1). Once the fault lifts, the outbox's retry meets that value and lands
//! it once: every member lists one channel.

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelRequest, CommunityRequest, IpcRequest};
use rekindle_netsim::{Fault, Writes};
use rekindle_types::dht_layout::member_record;

use crate::harness::{eventually, Community};
use crate::invite_unpublished::creator_record;

/// How many of `community`'s members list a channel named `name`, each.
async fn listed(community: &Community, list: &IpcRequest, name: &str) -> Vec<usize> {
    let needle = format!("\"name\":\"{name}\"");
    let mut counts = Vec::new();
    for node in &community.nodes {
        let channels = node.call(list.clone()).await.unwrap_or_default();
        counts.push(channels.to_string().matches(&needle).count());
    }
    counts
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_partially_written_op_is_readable_while_pending_and_lands_once() {
    let community = Community::of(3, 241).await;
    let record = creator_record(&community);
    let before = community
        .sim
        .network_get(&record, member_record::GOV_HEAD)
        .map(|v| v.seq);
    community.sim.fault(Fault::WritePartial {
        node: 0,
        of: Writes::Record(record.clone()),
        accepting: 2,
    });
    let create = IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::ChannelCreate {
        community: community.governance_key.clone(),
        name: "partial".into(),
        kind: "text".into(),
        category: None,
        topic: None,
        slowmode_seconds: 0,
        parent_voice_channel: None,
    }));
    let answer = community.nodes[0].call(create).await;
    assert!(answer.is_ok(), "a pending op is a success: {answer:?}");

    // Two replicas hold the new op-log head, below consensus, and a get
    // reaching them reads it while the writer still has it queued.
    let newer: Vec<u32> = community
        .sim
        .replica_values(&record, member_record::GOV_HEAD)
        .into_iter()
        .flatten()
        .map(|v| v.seq)
        .filter(|seq| before.is_none_or(|b| *seq > b))
        .collect();
    assert_eq!(newer.len(), 2, "only the accepting replicas: {newer:?}");
    let read = community
        .sim
        .network_get(&record, member_record::GOV_HEAD)
        .map(|v| v.seq);
    assert!(
        read > before,
        "a reader reaching them sees the op: {before:?} -> {read:?}"
    );

    community.sim.fault(Fault::Heal);
    let list = IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::ChannelList {
        community: community.governance_key.clone(),
    }));
    let converged = eventually(&community.sim, Duration::from_secs(900), || async {
        listed(&community, &list, "partial").await == vec![1, 1, 1]
    })
    .await;
    let counts = listed(&community, &list, "partial").await;
    assert!(converged, "every member lists the channel once: {counts:?}");
    // The outbox's retry lands the op-log head at consensus.
    let head_landed = || {
        let head = community
            .sim
            .network_get(&record, member_record::GOV_HEAD)
            .map(|v| v.seq);
        let held = community
            .sim
            .replica_values(&record, member_record::GOV_HEAD)
            .into_iter()
            .flatten()
            .filter(|v| Some(v.seq) == head)
            .count();
        (head, held)
    };
    let landed = eventually(&community.sim, Duration::from_secs(900), || async {
        let (head, held) = head_landed();
        head >= read && held >= 5
    })
    .await;
    assert!(landed, "the op-log head landed: {:?}", head_landed());
    assert_eq!(
        listed(&community, &list, "partial").await,
        vec![1, 1, 1],
        "still one channel each"
    );
}
