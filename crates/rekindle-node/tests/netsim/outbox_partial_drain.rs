//! Plan E3.3 (C9) `outbox_partial_drain`: a drain that commits part of its
//! batch, then hears "try again", and retries. The author queues forty
//! posts of 1,900 characters (a text body's cap is 2,000), so the drain
//! appends them as several page runs of one sealed 32,768 B page each
//! (`outbox_chain.rs` `append_fresh`), each run its page and an `M`/4 head
//! set. While the author's head sets reach only 2 replicas
//! (`Fault::WritePartial`), Veilid's `set_value` ends below consensus with
//! those replicas holding the new head, and the writer, with
//! `AllowOffline(false)`, stores nothing and hears `TryAgain` (0.5.8
//! `set_value.rs:711-738`, `local_record_store_interface.rs:60-63`): a
//! reader reaching them sees the first run committed while the writer's
//! drain stopped there. Once the fault lifts the stream retries the batch;
//! an entry's `streamSeq` is frozen at creation, so the retry appends no
//! entry twice. Afterwards a second reader holds each post once, its
//! cursor reached the author's whole chain (no history gap marker), and
//! the author's chain names exactly forty entries.

use std::time::Duration;

use rekindle_netsim::{Fault, Records, Writes};
use rekindle_types::dht_layout::member_record::OUTBOX;

use crate::harness::{eventually, Community};
use crate::invite_unpublished::creator_record;

/// The posts the author queues.
const POSTS: u8 = 40;

/// A body under the 2,000-character cap; forty fill several pages.
fn body(n: u8) -> String {
    format!("partial-{n:02}-{}", "x".repeat(1_900))
}

/// How many messages of node `i`'s general channel carry `body`, and
/// whether any row is a history gap marker.
async fn copies(community: &Community, i: usize, body: &str) -> (usize, bool) {
    let page = community.history(i, None, 100).await.unwrap_or_default();
    let count = page.iter().filter(|m| m["body"] == body).count();
    let gap = page.iter().any(|m| !m["historyGapThrough"].is_null());
    (count, gap)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_partially_committed_drain_retries_without_duplicates_or_gaps() {
    let mut community = Community::create(3, 517).await;
    community.join(1).await;
    community.join(2).await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    let record = creator_record(&community);
    community.sim.fault(Fault::WritePartial {
        node: 0,
        of: Writes::Subkey {
            subkey: OUTBOX,
            record: Some(Records::Key(record.clone())),
        },
        accepting: 2,
    });
    for n in 1..=POSTS {
        community
            .post(0, &body(n))
            .await
            .expect("a queued post is a success");
    }
    // The first run's head reaches the accepting replicas while the
    // writer's set ends below consensus.
    let partial = eventually(&community.sim, Duration::from_secs(300), || async {
        community
            .sim
            .replica_values(&record, OUTBOX)
            .into_iter()
            .flatten()
            .next()
            .is_some()
    })
    .await;
    assert!(partial, "a replica holds the drain's first committed head");

    community.sim.fault(Fault::Heal);
    let read_all = eventually(&community.sim, Duration::from_secs(1800), || async {
        let mut all = true;
        for n in 1..=POSTS {
            all &= copies(&community, 2, &body(n)).await.0 >= 1;
        }
        all
    })
    .await;
    assert!(read_all, "the second reader reads every post");
    for n in 1..=POSTS {
        let (count, gap) = copies(&community, 2, &body(n)).await;
        assert_eq!(count, 1, "post {n} is held once by the second reader");
        assert!(!gap, "the second reader's cursor reached the head, no gap");
        assert_eq!(
            copies(&community, 0, &body(n)).await.0,
            1,
            "post {n} is held once by its author"
        );
    }
    // The author's chain holds each entry once: its published head names
    // chain seq 40 as the newest (a re-appended entry would name more).
    let head = community
        .sim
        .network_get(&record, OUTBOX)
        .expect("the author's outbox head");
    let head = rekindle_codec::capnp_codec::chain::decode_chain_head(&head.data)
        .expect("a chain head")
        .expect("a chain with entries");
    assert_eq!(
        head.newest.last_seq,
        u64::from(POSTS),
        "forty entries, none appended twice"
    );
    assert!(
        head.newest.first_seq > 1,
        "the batch spans more than one page"
    );
}
