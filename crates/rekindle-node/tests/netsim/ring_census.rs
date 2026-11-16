//! The watch ring's census while a ring scenario runs (plan E3.2 S7
//! Verify, N = 40): every simulated minute one stderr line with the
//! watchers of each member record `M`, the replicas' watch refusals, and
//! each node's merged member count, so a run that stalls says where. A
//! wall-clock guard ends the test process with the last census when the
//! run passes its limit, since a stalled simulated clock prints nothing.

use std::io::Write;
use std::sync::{mpsc, Arc};

use parking_lot::Mutex;
use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_types::dht_layout::member_record::SUBKEY_COUNT;

use crate::harness::Community;

/// How long a ring scenario may run on the wall clock.
pub const WALL_LIMIT: Duration = Duration::from_secs(70 * 60);

/// One census line at simulated time `at`.
async fn line(community: &Community, at: Duration) -> String {
    let watched = community.sim.watchers_of_dflt(SUBKEY_COUNT);
    let mut watchers: Vec<usize> = watched
        .values()
        .map(std::collections::BTreeSet::len)
        .collect();
    watchers.sort_unstable();
    let mut merged = Vec::with_capacity(community.nodes.len());
    for node in &community.nodes {
        let info = tokio::time::timeout(
            Duration::from_secs(5),
            node.call(IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityInfo {
                    governance_key: community.governance_key.clone(),
                },
            ))),
        )
        .await;
        merged.push(match info {
            Ok(Ok(v)) => v["member_count"].to_string(),
            Ok(Err(_)) => "-".to_owned(),
            Err(_) => "stuck".to_owned(),
        });
    }
    format!(
        "census t={}s: {} records watched, watchers per M {watchers:?}, refusals {}, merged members {merged:?}",
        at.as_secs(),
        watched.len(),
        community.sim.watch_refusals(),
    )
}

/// Prints a census line every simulated 60 s, forever; the caller races it
/// against the scenario.
pub async fn every_minute(community: &Community, last: Arc<Mutex<String>>) {
    let start = tokio::time::Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.tick().await;
    loop {
        tick.tick().await;
        let census = line(community, start.elapsed()).await;
        writeln!(std::io::stderr(), "{census}").expect("stderr");
        *last.lock() = census;
    }
}

/// The wall-clock guard: unless `done` is dropped or sent to within
/// [`WALL_LIMIT`], prints the last census and exits the test process
/// with a failure.
pub fn guard(last: Arc<Mutex<String>>) -> mpsc::Sender<()> {
    let (done, wait) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        if let Err(mpsc::RecvTimeoutError::Timeout) = wait.recv_timeout(WALL_LIMIT) {
            let census = last.lock().clone();
            writeln!(
                std::io::stderr(),
                "ring scenario passed its {}-minute wall-clock limit; last {census}",
                WALL_LIMIT.as_secs() / 60
            )
            .expect("stderr");
            std::process::exit(101);
        }
    });
    done
}
