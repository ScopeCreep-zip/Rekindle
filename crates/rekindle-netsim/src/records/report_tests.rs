//! Create checks, inspect scopes and `offline_subkeys` (plan E2.B S10),
//! against the Veilid lines [`super::dht`] and [`super::validate_schema`]
//! cite.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{DhtBackend, DhtErrorKind, RecordSchema, ReportScope, SchemaMember};

use super::dht::SimDht;
use crate::{Netsim, Node};

fn dht(sim: &Netsim, node: &Node) -> SimDht {
    SimDht::new(node.index, Arc::clone(&sim.net), Arc::clone(&node.rng))
}

fn members(n: usize) -> Vec<SchemaMember> {
    (0..n)
        .map(|i| {
            let mut key = [0u8; 32];
            key[..8].copy_from_slice(&(i as u64).to_le_bytes());
            SchemaMember { key, subkeys: 1 }
        })
        .collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn create_checks_members_writers_and_owner_subkeys() {
    let (sim, nodes) = Netsim::nodes(1, 31);
    let _pump = sim.start();
    let a = dht(&sim, &nodes[0]);
    let refused = |schema: RecordSchema| {
        let a = &a;
        async move {
            matches!(a.create(&schema, None).await, Err(e) if e.kind == DhtErrorKind::InvalidArgument)
        }
    };
    assert!(refused(RecordSchema::Dflt { owner_subkeys: 0 }).await);
    assert!(
        refused(RecordSchema::Smpl {
            owner_subkeys: 0,
            members: members(257),
        })
        .await
    );
    // 256 members with subkeys and the owner: 257 writers.
    assert!(
        refused(RecordSchema::Smpl {
            owner_subkeys: 1,
            members: members(256),
        })
        .await
    );
    for schema in [
        RecordSchema::Smpl {
            owner_subkeys: 1,
            members: members(255),
        },
        RecordSchema::Smpl {
            owner_subkeys: 0,
            members: members(256),
        },
    ] {
        assert!(a.create(&schema, None).await.is_ok());
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn update_scopes_start_from_the_local_result() {
    let (sim, nodes) = Netsim::nodes(1, 32);
    let _pump = sim.start();
    let a = dht(&sim, &nodes[0]);
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap()
        .key;
    a.set(&key, 0, vec![1], None).await.unwrap();
    let seqs = |scope| {
        let (a, key) = (&a, &key);
        async move {
            let r = a.inspect(key, None, scope).await.unwrap();
            (r.local_seqs, r.network_seqs)
        }
    };
    assert_eq!(seqs(ReportScope::Local).await, (vec![Some(0)], vec![None]));
    assert_eq!(
        seqs(ReportScope::UpdateGet).await,
        (vec![Some(0)], vec![Some(0)])
    );
    // A simulated set: the local seq + 1, which the network's 0 does not beat.
    assert_eq!(
        seqs(ReportScope::UpdateSet).await,
        (vec![Some(1)], vec![Some(1)])
    );
    // Only UpdateSet bumps (`inspect_record.rs:208-215`); a Sync scope
    // reports the network's own seq.
    assert_eq!(
        seqs(ReportScope::SyncSet).await,
        (vec![Some(0)], vec![Some(0)])
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_set_in_flight_is_an_offline_subkey() {
    let (sim, nodes) = Netsim::nodes(1, 33);
    let _pump = sim.start();
    let a = dht(&sim, &nodes[0]);
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 2 }, None)
        .await
        .unwrap()
        .key;
    let b = dht(&sim, &nodes[0]);
    let k = key.clone();
    let set = tokio::spawn(async move { b.set(&k, 1, vec![1], None).await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    let during = a.inspect(&key, None, ReportScope::Local).await.unwrap();
    assert_eq!(during.offline_subkeys, vec![1]);
    set.await.unwrap().unwrap();
    let after = a.inspect(&key, None, ReportScope::Local).await.unwrap();
    assert!(after.offline_subkeys.is_empty());
}
