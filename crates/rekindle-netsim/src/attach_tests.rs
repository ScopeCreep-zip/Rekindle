//! Attachment, readiness and the messages of a detached node (plan E2.B S9)
//! against Veilid.

use std::time::Duration;

use rekindle_records::port::{
    PeerTarget, PortError, PortEvent, RecordSchema, RouteClass, SetOutcome,
};
use rekindle_types::config::SafetyProfile;

use super::tests::{attached, recv};
use super::*;

/// The attachment updates `node` hears until one reaches `last`.
async fn states_until(node: &mut Node, last: &str) -> Vec<(String, bool, bool)> {
    let mut seen = Vec::new();
    loop {
        if let PortEvent::Attachment {
            state,
            is_attached,
            public_internet_ready,
            ..
        } = recv(node).await
        {
            let done = state == last;
            seen.push((state, is_attached, public_internet_ready));
            if done {
                return seen;
            }
        }
    }
}

fn ready(sim: &Netsim, node: usize) -> bool {
    *sim.net.inner.lock().nodes[node].ready.borrow()
}

/// Node 1's general route, imported at node 0.
async fn route_to_1(sim: &Netsim, nodes: &mut [Node]) -> PeerTarget {
    nodes[1].ports.routes.want_own();
    let blob = nodes[1]
        .ports
        .routes
        .available_own(RouteClass::General)
        .await;
    attached(&mut nodes[0]).await;
    attached(&mut nodes[1]).await;
    sim.quiesce().await;
    nodes[0].ports.routes.import(&blob).unwrap()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn attachment_steps_through_veilid_s_states() {
    let (sim, mut nodes) = Netsim::nodes(1, 21);
    let _pump = sim.start();
    let names = |seen: &[(String, bool, bool)]| -> Vec<String> {
        seen.iter().map(|s| s.0.clone()).collect()
    };
    let up = states_until(&mut nodes[0], "attached_full").await;
    assert_eq!(
        names(&up),
        [
            "attaching",
            "attached_weak",
            "attached_fair",
            "attached_good",
            "attached_strong",
            "attached_full"
        ]
    );
    // `is_attached` from AttachedWeak (`veilid_state.rs:31-41`).
    assert!(!up[0].1);
    assert!(up[1..].iter().all(|s| s.1));
    assert!(ready(&sim, 0));
    sim.fault(Fault::Detach {
        node: 0,
        detached: true,
    });
    let down = states_until(&mut nodes[0], "detached").await;
    assert_eq!(names(&down), ["detaching", "detached"]);
    assert!(down.iter().all(|s| !s.1 && !s.2));
    assert!(!ready(&sim, 0));
    sim.fault(Fault::Detach {
        node: 0,
        detached: false,
    });
    let back = states_until(&mut nodes[0], "attached_good").await;
    assert_eq!(back.first().map(|s| s.0.as_str()), Some("attaching"));
    assert!(ready(&sim, 0));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_detached_node_neither_sends_nor_receives() {
    let (sim, mut nodes) = Netsim::nodes(2, 22);
    let _pump = sim.start();
    let target = route_to_1(&sim, &mut nodes).await;
    let profile = SafetyProfile::default_dht();
    // In flight when the receiver detaches: lost.
    let messages = nodes[0].ports.messages.clone();
    messages.send(&target, &profile, vec![1]).await.unwrap();
    sim.fault(Fault::Detach {
        node: 1,
        detached: true,
    });
    // Sent to a detached node: dispatched, then lost; a call times out.
    messages.send(&target, &profile, vec![2]).await.unwrap();
    let started = tokio::time::Instant::now();
    assert!(matches!(
        messages.call(&target, &profile, vec![3]).await,
        Err(PortError::Send(_))
    ));
    assert!(started.elapsed() >= call_timeout(&profile));
    sim.quiesce().await;
    while let Ok(event) = nodes[1].events.try_recv() {
        assert!(
            matches!(event, PortEvent::Attachment { .. }),
            "a detached node got {event:?}"
        );
    }
    // A detached sender: its RPC processor is down (TryAgain → Send).
    sim.fault(Fault::Detach {
        node: 0,
        detached: true,
    });
    assert!(matches!(
        messages.send(&target, &profile, vec![4]).await,
        Err(PortError::Send(_))
    ));
    assert!(matches!(
        messages.call(&target, &profile, vec![5]).await,
        Err(PortError::Send(_))
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_internet_ready_flap_gates_the_dht_while_attached() {
    let (sim, mut nodes) = Netsim::nodes(1, 23);
    let _pump = sim.start();
    attached(&mut nodes[0]).await;
    let pool = nodes[0].ports.records.start_session().unwrap();
    let mut settled = pool.settled();
    let (lease, key, _) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    sim.fault(Fault::InternetReady {
        node: 0,
        ready: false,
    });
    let PortEvent::Attachment {
        state,
        is_attached,
        public_internet_ready,
        ..
    } = recv(&mut nodes[0]).await
    else {
        panic!("expected an attachment update");
    };
    assert_eq!(
        (state.as_str(), is_attached, public_internet_ready),
        ("attached_full", true, false)
    );
    assert!(!ready(&sim, 0));
    assert_eq!(
        pool.set_durable(lease, 0, b"flap".to_vec(), None)
            .await
            .unwrap(),
        SetOutcome::Offline
    );
    // Back within the flap: the pool's re-push lands the held write.
    sim.fault(Fault::InternetReady {
        node: 0,
        ready: true,
    });
    let landed = tokio::time::timeout(Duration::from_secs(120), settled.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(landed.landed && ready(&sim, 0));
    assert_eq!(sim.network_get(&key, 0).unwrap().data, b"flap");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_reply_to_an_expired_call_is_ignored() {
    let (sim, mut nodes) = Netsim::nodes(2, 24);
    let _pump = sim.start();
    let target = route_to_1(&sim, &mut nodes).await;
    let profile = SafetyProfile::default_dht();
    let (us, them) = nodes.split_at_mut(1);
    let caller = us[0].ports.messages.call(&target, &profile, vec![1]);
    let callee = async {
        loop {
            if let PortEvent::AppCall { call_id, .. } = recv(&mut them[0]).await {
                return call_id;
            }
        }
    };
    let (answer, call_id) = tokio::join!(caller, callee);
    assert!(answer.is_err(), "nobody answered in time");
    assert!(matches!(
        nodes[1].ports.messages.reply(call_id, vec![2]).await,
        Err(PortError::Send(_))
    ));
}

#[test]
fn the_call_timeout_follows_veilid_s_hop_rule() {
    let mut profile = SafetyProfile::default_dht();
    profile.hop_count = 0;
    // Floor 3 hops: 5 s + 500 ms × (6 − 4).
    assert_eq!(call_timeout(&profile), Duration::from_secs(6));
    profile.hop_count = 4;
    assert_eq!(call_timeout(&profile), Duration::from_secs(7));
}
