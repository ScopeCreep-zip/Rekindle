//! Plan E3.4 OW2 (lead decision, Linux 60-min finding): the own-write
//! republish inspects each record once per round and re-sends only what
//! the network holds behind our seq. A login's prekey record (a head and
//! 100 one-time slots) re-sent every subkey every round under OW1, ~5.5k
//! DHT writes per app per day; landed, it now costs one inspect a round.

use std::time::Duration;

use rekindle_types::dht_layout::prekeys::{slot_subkey, HEAD, SLOT_COUNT};

use crate::harness::{eventually, start};
use crate::pair::LIMIT;

/// The republish's rounds within its first hour: 60 s, then doubling
/// waits (60+120+240+480+960+1800 s ≤ 3_660 s).
const ROUNDS_IN_FIRST_HOUR: u64 = 6;

/// Inspects of the record the 62 minutes may see: one per round, plus one
/// this scenario measures and OW2 does not yet attribute (open, reported
/// to the lead): every inspect the netsim counts, any scope or caller.
const MOST_INSPECTS: u64 = ROUNDS_IN_FIRST_HOUR + 1;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_landed_prekey_record_costs_one_inspect_a_round_and_no_re_send() {
    let (sim, mut nodes) = start(3, 3147).await;
    nodes[0].unlock_new_identity("Ann").await;
    assert!(
        eventually(&sim, LIMIT, || async {
            nodes[0].prekey_status().await.record_key.is_some()
        })
        .await,
        "the unlock holds a prekey record"
    );
    let record = nodes[0]
        .prekey_status()
        .await
        .record_key
        .expect("a prekey record");
    assert!(
        eventually(&sim, LIMIT, || async {
            sim.network_get(&record, HEAD).is_some()
                && (0..SLOT_COUNT).all(|i| sim.network_get(&record, slot_subkey(i)).is_some())
        })
        .await,
        "the prekey record is published in full"
    );
    let (inspects, resends) = (sim.app_inspects(0, &record), sim.app_resends(0));
    tokio::time::sleep(Duration::from_secs(62 * 60)).await;
    let inspected = sim.app_inspects(0, &record) - inspects;
    let resent = sim.app_resends(0) - resends;
    tracing::info!(inspected, resent, "prekey record over 62 min: inspects and re-sends");
    assert_eq!(
        resent, 0,
        "every own write landed: the republish re-sends none of them"
    );
    assert!(
        (1..=MOST_INSPECTS).contains(&inspected),
        "{inspected} inspects of the prekey record in 62 min: one per round, at most {MOST_INSPECTS}"
    );
}
