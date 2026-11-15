//! The Host's wake trigger (plan VU decision 8, synthesis #7,
//! `host/wake.rs`): a change missed while the machine slept surfaces on
//! waking through the Host's tick-lag detector (a 5 s monotonic tick whose
//! wall-clock gap is 30 s or more), not at the watch's fallback inspection,
//! which after a quiet spell runs only every 300 s (0.5.8
//! `outbound_watch_state.rs:141-146`, `max_watch_inspect_interval_ms`
//! `veilid_config.rs:1018`) and 0.5.8 has no tick-lag sweep. `Fault::Sleep`
//! freezes the node: its calls park, its wall clock stands still and jumps
//! across the sleep on waking.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::presence::SessionStatus;

use crate::harness::{eventually, start};
use crate::pair::{befriend, friends, key, LIMIT};
use crate::watch_fallback::profile_key;

/// Max goes Invisible, so his heartbeat stops; Nia watches his profile
/// through the quiet spell until her watch's
/// fallback interval is 300 s; the backoff to 300 s is itself a fallback
/// inspection, so the next one is 300 s off. Her machine then sleeps a
/// minute while Max renames himself; the push is lost. On waking she must
/// show the rename within a wake tick and its heal, long before the
/// fallback inspection (still ≥ 240 s away) could bring it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_change_missed_in_a_sleep_arrives_on_waking_before_the_fallback() {
    let (sim, mut nodes) = start(2, 133).await;
    for (node, name) in nodes.iter_mut().zip(["Max", "Nia"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let max = key(&nodes[0]);
    let profile = profile_key(&nodes[0]).await;
    let shows = |name: &'static str| {
        let (nia, max) = (&nodes[1], max.clone());
        async move {
            friends(nia)
                .await
                .iter()
                .any(|f| f.public_key == max && f.display_name == name)
        }
    };
    assert!(
        eventually(&sim, LIMIT, || shows("Max")).await,
        "Nia shows Max"
    );
    // Invisible writes its status once and then no heartbeat (the
    // publisher's `hides_presence`), so nothing pushes Max's profile to Nia
    // and her watch's fallback backs off.
    nodes[0]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Invisible,
        })
        .await;
    let ceiling = Some(Duration::from_secs(300));
    assert!(
        eventually(&sim, Duration::from_secs(1800), || async {
            sim.fallback_interval(1, &profile) == ceiling
        })
        .await,
        "a quiet spell backs the fallback off to 300 s: {:?}",
        sim.fallback_interval(1, &profile)
    );
    sim.fault(Fault::Sleep {
        node: 1,
        asleep: true,
    });
    nodes[0]
        .ok(IpcRequest::ProfileSetNickname {
            display_name: "Maxine".into(),
        })
        .await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(!shows("Maxine").await, "the push was lost in the sleep");
    sim.fault(Fault::Sleep {
        node: 1,
        asleep: false,
    });
    assert!(
        eventually(&sim, Duration::from_secs(20), || shows("Maxine")).await,
        "Nia shows the rename within a wake tick and its heal"
    );
}
