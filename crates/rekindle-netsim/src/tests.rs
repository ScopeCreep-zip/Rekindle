//! The fakes against the Veilid behaviour the app depends on.

use std::time::Duration;

use rekindle_records::port::{
    PortError, PortEvent, RecordSchema, ReportScope, RouteClass, RouteState, SchemaMember,
    SetOutcome,
};
use rekindle_types::config::SafetyProfile;

use super::*;

fn keypair(seed: u8) -> (String, [u8; 32]) {
    let signing = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
    (
        rekindle_crypto::keypair_text::keypair_text_from_signing(&signing),
        signing.verifying_key().to_bytes(),
    )
}

async fn recv(node: &mut Node) -> PortEvent {
    tokio::time::timeout(Duration::from_secs(5), node.events.recv())
        .await
        .expect("an event")
        .expect("open")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_record_is_local_until_its_first_write_lands() {
    let (sim, nodes) = Netsim::nodes(2, 1);
    let (a, b) = (
        nodes[0].ports.records.start_session().unwrap(),
        nodes[1].ports.records.start_session().unwrap(),
    );
    let (lease, key, _) = a
        .create(&RecordSchema::Dflt { owner_subkeys: 2 }, None)
        .await
        .unwrap();
    assert!(matches!(
        b.acquire(&key, None).await,
        Err(PortError::Unreachable(_))
    ));
    assert_eq!(
        a.set(lease, 0, b"v".to_vec(), None).await.unwrap(),
        SetOutcome::Landed
    );
    let theirs = b.acquire(&key, None).await.unwrap();
    let value = b.get(theirs, 0, true).await.unwrap().unwrap();
    assert_eq!((value.seq, value.data.as_slice()), (0, &b"v"[..]));
    assert_eq!(sim.network_get(&key, 0), Some(value));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn only_the_schema_s_writers_write() {
    let (_sim, nodes) = Netsim::nodes(1, 2);
    let pool = nodes[0].ports.records.start_session().unwrap();
    let (member, member_key) = keypair(9);
    let (other, other_key) = keypair(10);
    let schema = RecordSchema::Smpl {
        owner_subkeys: 1,
        members: vec![
            SchemaMember {
                key: member_key,
                subkeys: 1,
            },
            SchemaMember {
                key: other_key,
                subkeys: 1,
            },
        ],
    };
    let (lease, _, _) = pool.create(&schema, None).await.unwrap();
    assert_eq!(
        pool.set(lease, 1, b"mine".to_vec(), Some(&member))
            .await
            .unwrap(),
        SetOutcome::Landed
    );
    assert!(matches!(
        pool.set(lease, 2, b"theirs".to_vec(), Some(&member)).await,
        Err(PortError::Dht(_))
    ));
    assert!(matches!(
        pool.set(lease, 0, b"owner".to_vec(), Some(&other)).await,
        Err(PortError::Dht(_))
    ));
    let cap = pool.subkey_cap(lease).unwrap();
    assert!(matches!(
        pool.set(lease, 0, vec![0; cap + 1], None).await,
        Err(PortError::SubkeyTooLarge { .. })
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_newer_network_value_supersedes_and_becomes_local() {
    let (_sim, nodes) = Netsim::nodes(2, 3);
    let (a, b) = (
        nodes[0].ports.records.start_session().unwrap(),
        nodes[1].ports.records.start_session().unwrap(),
    );
    let (lease, key, owner) = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    a.set(lease, 0, b"first".to_vec(), None).await.unwrap();
    let theirs = b.acquire(&key, Some(&owner)).await.unwrap();
    match b.set(theirs, 0, b"second".to_vec(), None).await.unwrap() {
        SetOutcome::Superseded(newer) => assert_eq!(newer.data, b"first"),
        other => panic!("expected a supersede, got {other:?}"),
    }
    assert_eq!(
        b.set(theirs, 0, b"second".to_vec(), None).await.unwrap(),
        SetOutcome::Landed
    );
    let report = a
        .inspect(lease, None, ReportScope::UpdateGet)
        .await
        .unwrap();
    assert_eq!(report.local_seqs, vec![Some(0)]);
    assert_eq!(report.network_seqs, vec![Some(1)]);
    assert_eq!(report.newer_on_network(), vec![0]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_watcher_hears_another_node_s_write() {
    let (sim, mut nodes) = Netsim::nodes(2, 4);
    let _pump = sim.start();
    let (a, b) = (
        nodes[0].ports.records.start_session().unwrap(),
        nodes[1].ports.records.start_session().unwrap(),
    );
    let (lease, key, _) = a
        .create(&RecordSchema::Dflt { owner_subkeys: 3 }, None)
        .await
        .unwrap();
    a.set(lease, 0, b"seed".to_vec(), None).await.unwrap();
    let theirs = b.acquire(&key, None).await.unwrap();
    b.watch(theirs, &[2]).await.unwrap();
    let _ = recv(&mut nodes[1]).await; // its attachment
    a.set(lease, 2, b"news".to_vec(), None).await.unwrap();
    match recv(&mut nodes[1]).await {
        PortEvent::ValueChange(change) => {
            assert_eq!(
                (change.key.as_str(), change.subkeys.as_slice()),
                (key.as_str(), &[2][..])
            );
            assert_eq!(change.value.unwrap().data, b"news");
        }
        other => panic!("expected a value change, got {other:?}"),
    }
    b.release(theirs).await;
    a.set(lease, 2, b"later".to_vec(), None).await.unwrap();
    sim.quiesce().await;
    assert!(
        nodes[1].events.try_recv().is_err(),
        "a released record's watch ends"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn messages_and_calls_reach_the_route_s_owner() {
    let (sim, mut nodes) = Netsim::nodes(2, 5);
    let pump = sim.start();
    nodes[1].ports.routes.want_own();
    let blob = tokio::time::timeout(
        Duration::from_secs(5),
        nodes[1].ports.routes.available_own(RouteClass::General),
    )
    .await
    .unwrap();
    let _ = recv(&mut nodes[1]).await; // its attachment
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    let profile = SafetyProfile::default_dht();
    nodes[0]
        .ports
        .messages
        .send(&target, &profile, b"hello".to_vec())
        .await
        .unwrap();
    assert!(
        matches!(recv(&mut nodes[1]).await, PortEvent::AppMessage { message } if message == b"hello")
    );

    let caller = nodes[0].ports.messages.clone();
    let call = tokio::spawn(async move { caller.call(&target, &profile, b"ping".to_vec()).await });
    let PortEvent::AppCall { call_id, message } = recv(&mut nodes[1]).await else {
        panic!("expected a call");
    };
    assert_eq!(message, b"ping");
    nodes[1]
        .ports
        .messages
        .reply(call_id, b"pong".to_vec())
        .await
        .unwrap();
    assert_eq!(call.await.unwrap().unwrap(), b"pong");
    assert!(matches!(
        nodes[0]
            .ports
            .messages
            .send(
                &rekindle_records::port::PeerTarget::new("gone".into()),
                &SafetyProfile::default_dht(),
                vec![1]
            )
            .await,
        Err(PortError::RouteUnusable(_))
    ));
    assert!(matches!(
        nodes[0]
            .ports
            .messages
            .send(
                &nodes[0].ports.routes.import(&blob).unwrap(),
                &SafetyProfile::default_dht(),
                vec![0; rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN + 1]
            )
            .await,
        Err(PortError::MessageTooLarge { .. })
    ));
    pump.abort();
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unanswered_call_times_out_and_a_partition_drops() {
    let (sim, mut nodes) = Netsim::nodes(2, 6);
    let _pump = sim.start();
    nodes[1].ports.routes.want_own();
    let blob = nodes[1]
        .ports
        .routes
        .available_own(RouteClass::General)
        .await;
    let _ = recv(&mut nodes[1]).await;
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    let profile = SafetyProfile::default_dht();
    sim.fault(Fault::Unanswered {
        node: 1,
        unanswered: true,
    });
    let started = tokio::time::Instant::now();
    assert!(nodes[0]
        .ports
        .messages
        .call(&target, &profile, vec![1])
        .await
        .is_err());
    assert!(started.elapsed() >= CALL_TIMEOUT);
    sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1],
    });
    nodes[0]
        .ports
        .messages
        .send(&target, &profile, vec![2])
        .await
        .unwrap();
    sim.quiesce().await;
    while let Ok(event) = nodes[1].events.try_recv() {
        assert!(
            !matches!(event, PortEvent::AppMessage { .. }),
            "partitioned"
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_offline_durable_write_is_held_and_the_pool_re_pushes_it() {
    let (sim, nodes) = Netsim::nodes(1, 7);
    let pool = nodes[0].ports.records.start_session().unwrap();
    let mut settled = pool.settled();
    let (lease, key, _) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    // The pool's one retry layer answers a "try again" read.
    sim.fault(Fault::TryAgain { node: 0, calls: 1 });
    assert_eq!(pool.get(lease, 0, true).await.unwrap(), None);
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });
    assert_eq!(
        pool.set_durable(lease, 0, b"held".to_vec(), None)
            .await
            .unwrap(),
        SetOutcome::Offline
    );
    assert_eq!(pool.held_writes(), vec![(key.clone(), 0)]);
    // The pool's re-push lands it once the network is back.
    sim.fault(Fault::Heal);
    let landed = tokio::time::timeout(Duration::from_secs(120), settled.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(landed.landed);
    assert!(pool.held_writes().is_empty());
    assert_eq!(sim.network_get(&key, 0).unwrap().data, b"held");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn route_death_reaches_the_owner_and_the_importers() {
    let (sim, mut nodes) = Netsim::nodes(2, 8);
    let _pump = sim.start();
    nodes[1].ports.routes.want_own();
    let blob = nodes[1]
        .ports
        .routes
        .available_own(RouteClass::General)
        .await;
    let _ = recv(&mut nodes[0]).await;
    let _ = recv(&mut nodes[1]).await;
    nodes[0].ports.routes.import(&blob).unwrap();
    sim.fault(Fault::RouteDeath { node: 1 });
    let PortEvent::RouteChange { dead, .. } = recv(&mut nodes[1]).await else {
        panic!("expected a route change");
    };
    assert!(!dead.is_empty());
    assert!(
        matches!(recv(&mut nodes[0]).await, PortEvent::RouteChange { dead_remote, .. } if !dead_remote.is_empty())
    );
    // The owner forgets the dead route and allocates a new one, as the
    // transport's dispatch has it do.
    nodes[1].ports.routes.on_dead_own(&dead);
    let fresh = tokio::time::timeout(
        Duration::from_secs(30),
        nodes[1].ports.routes.available_own(RouteClass::General),
    )
    .await
    .unwrap();
    assert_ne!(fresh, blob);
    assert!(matches!(
        *nodes[1]
            .ports
            .routes
            .own_state(RouteClass::General)
            .borrow(),
        RouteState::Available { .. }
    ));
}

/// The delivery order is the seed's: one seed, one trace; another seed,
/// another order.
#[test]
fn the_seed_fixes_the_delivery_order() {
    fn run(seed: u64) -> Trace {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap();
        runtime.block_on(async {
            let (sim, mut nodes) = Netsim::nodes(4, seed);
            let _pump = sim.start();
            let mut targets = Vec::new();
            for node in &nodes {
                node.ports.routes.want_own();
                targets.push(node.ports.routes.available_own(RouteClass::General).await);
            }
            sim.quiesce().await;
            for (from, sender) in nodes.iter().enumerate() {
                for (to, blob) in targets.iter().enumerate() {
                    if from != to {
                        let target = sender.ports.routes.import(blob).unwrap();
                        sender
                            .ports
                            .messages
                            .send(
                                &target,
                                &SafetyProfile::default_dht(),
                                from.to_le_bytes().to_vec(),
                            )
                            .await
                            .unwrap();
                    }
                }
            }
            sim.quiesce().await;
            for node in &mut nodes {
                while node.events.try_recv().is_ok() {}
            }
            sim.trace()
        })
    }
    assert_eq!(run(11).hash(), run(11).hash());
    assert_ne!(run(11).hash(), run(12).hash());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn outside_writes_check_the_writer_and_a_conflict_keeps_its_seq() {
    let (sim, nodes) = Netsim::nodes(2, 9);
    let (a, b) = (
        nodes[0].ports.records.start_session().unwrap(),
        nodes[1].ports.records.start_session().unwrap(),
    );
    let (lease, key, owner) = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    a.set(lease, 0, b"first".to_vec(), None).await.unwrap();
    let theirs = b.acquire(&key, None).await.unwrap();
    assert_eq!(
        b.get(theirs, 0, true).await.unwrap().unwrap().data,
        b"first"
    );

    let (stranger, _) = keypair(77);
    assert!(matches!(
        sim.forge_write(&key, 0, b"x".to_vec(), &stranger),
        Err(PortError::NotWritable(_))
    ));

    sim.conflict_write(&key, 0, b"rival".to_vec(), &owner)
        .unwrap();
    // The replicas the first fanout reached keep "first"; the rest took
    // the rival at the same seq.
    let held: Vec<(u32, Vec<u8>)> = sim
        .replica_values(&key, 0)
        .into_iter()
        .flatten()
        .map(|v| (v.seq, v.data))
        .collect();
    assert_eq!(held.len(), 10);
    assert!(held.contains(&(0, b"rival".to_vec())));
    assert!(held.contains(&(0, b"first".to_vec())));
    // Both nodes keep the value they hold at that seq.
    assert_eq!(
        b.get(theirs, 0, true).await.unwrap().unwrap().data,
        b"first"
    );

    sim.forge_write(&key, 0, b"forged".to_vec(), &owner)
        .unwrap();
    let forged = b.get(theirs, 0, true).await.unwrap().unwrap();
    assert_eq!((forged.seq, forged.data.as_slice()), (1, &b"forged"[..]));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_creator_s_owner_key_rewrites_and_a_crash_loses_held_writes() {
    let (sim, nodes) = Netsim::nodes(1, 10);
    let pool = nodes[0].ports.records.start_session().unwrap();
    let (lease, key, owner) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 2 }, None)
        .await
        .unwrap();
    assert_eq!(sim.owner_keypair(&key), Some(owner.clone()));
    assert_eq!(sim.owner_keypair("VLD0:unknown"), None);
    pool.set(lease, 0, b"honest".to_vec(), None).await.unwrap();
    sim.forge_write(&key, 0, b"rewritten".to_vec(), &owner)
        .unwrap();
    assert_eq!(sim.network_get(&key, 0).unwrap().data, b"rewritten");

    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });
    assert_eq!(
        pool.set_durable(lease, 1, b"held".to_vec(), None)
            .await
            .unwrap(),
        SetOutcome::Offline
    );
    sim.lose_held(0);
    assert!(pool.held_writes().is_empty());
    sim.fault(Fault::Heal);
    assert_eq!(sim.network_get(&key, 1), None);
}

/// A friend invite's request inbox (plan E2.5, D17): once the inviter's
/// owner write puts it on the network, the invite's one-time writer writes
/// the request subkey from another node, and nobody else can — not
/// another keypair, and not the inviter's owner key.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn only_the_one_time_writer_writes_a_request_inbox() {
    use rekindle_types::dht_layout::request_inbox;
    let (_sim, nodes) = Netsim::nodes(3, 3);
    let (inviter, redeemer, stranger) = (
        nodes[0].ports.records.start_session().unwrap(),
        nodes[1].ports.records.start_session().unwrap(),
        nodes[2].ports.records.start_session().unwrap(),
    );
    let (writer, writer_key) = keypair(21);
    let (other, _) = keypair(22);
    let schema = rekindle_records::schema::request_inbox(&writer_key);
    let (lease, key, _) = inviter.create(&schema, None).await.unwrap();
    assert!(matches!(
        redeemer.acquire(&key, None).await,
        Err(PortError::Unreachable(_))
    ));
    inviter
        .set(
            lease,
            request_inbox::OWNER_SUBKEY,
            request_inbox::OPENED.to_vec(),
            None,
        )
        .await
        .unwrap()
        .require_stored(request_inbox::OWNER_SUBKEY)
        .unwrap();
    assert!(matches!(
        redeemer
            .write_once(
                &key,
                request_inbox::REQUEST,
                b"request".to_vec(),
                Some(&writer)
            )
            .await,
        Ok(SetOutcome::Landed)
    ));
    assert!(matches!(
        stranger
            .write_once(&key, request_inbox::REQUEST, b"spam".to_vec(), Some(&other))
            .await,
        Err(PortError::Dht(_))
    ));
    assert!(matches!(
        inviter
            .set(lease, request_inbox::REQUEST, b"owner".to_vec(), None)
            .await,
        Err(PortError::Dht(_))
    ));
    let theirs = stranger.acquire(&key, None).await.unwrap();
    let value = stranger
        .get(theirs, request_inbox::REQUEST, true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(value.data, b"request");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_set_waits_for_consensus_and_an_offline_set_fails_at_once() {
    let (sim, nodes) = Netsim::nodes(1, 11);
    let pool = nodes[0].ports.records.start_session().unwrap();
    let (lease, key, _) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 2 }, None)
        .await
        .unwrap();
    let start = tokio::time::Instant::now();
    let (first, second) = tokio::join!(
        pool.set(lease, 0, b"a".to_vec(), None),
        pool.set(lease, 1, b"b".to_vec(), None),
    );
    assert_eq!(first.unwrap(), SetOutcome::Landed);
    assert_eq!(second.unwrap(), SetOutcome::Landed);
    let took = start.elapsed();
    assert!(
        took >= latency::SET.min && took <= latency::SET.max,
        "two subkeys' sets overlap: {took:?}"
    );
    assert_eq!(sim.network_get(&key, 1).unwrap().data, b"b");
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });
    let start = tokio::time::Instant::now();
    assert_eq!(
        pool.set(lease, 0, b"c".to_vec(), None).await.unwrap(),
        SetOutcome::Offline
    );
    assert_eq!(start.elapsed(), Duration::ZERO);
}
