use std::sync::Arc;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_ipc::{Caller, HostApi};

use super::*;
use crate::host::{claim, Host, HostConfig, HostStores, TransportSource};

/// A host on one simulated node, with an identity unlocked and `peer`
/// an accepted friend.
async fn host_on_sim(dir: &tempfile::TempDir, peer: &str) -> (rekindle_netsim::Netsim, Arc<Host>) {
    let (sim, mut nodes) = rekindle_netsim::Netsim::nodes(1, 81);
    let node = nodes.remove(0);
    let root = rekindle_db::paths::DataRoot::under(dir.path());
    let storage_dir = root.data.join("veilid").display().to_string();
    let host = Host::start(HostConfig {
        root: claim(root).unwrap(),
        stores: HostStores::default(),
        transport: TransportSource::Ports {
            config: rekindle_types::config::TransportConfig {
                storage_dir,
                ..Default::default()
            },
            ports: node.ports,
            events: node.events,
        },
        audio: rekindle_types::audio::AudioIo::HEADLESS,
        serve_bus: false,
        clock: Arc::clone(&node.clock) as Arc<dyn rekindle_utils::Clock>,
        rng: Arc::clone(&node.rng) as Arc<dyn rekindle_utils::RandomSource>,
        idle: Arc::clone(&node.idle) as Arc<dyn rekindle_utils::IdleSource>,
        server_query: std::sync::Arc::new(rekindle_game_detect::server_query::UdpServerQuery),
        shell: None,
    })
    .await
    .unwrap();
    let _pump = sim.start();
    sim.quiesce().await;
    let created = host
        .call(
            Caller::shell(),
            IpcRequest::IdentityCreate {
                display_name: Some("Ann".into()),
                passphrase: rekindle_test_fixtures::PASSPHRASE.into(),
            },
        )
        .await;
    assert!(
        !matches!(created, rekindle_ipc::protocol::IpcResponse::Error { .. }),
        "{created:?}"
    );
    let owner = host.ctx.require_owner().unwrap();
    let p = peer.to_string();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::friends::insert_accepted(conn, &owner, &p, "Bob", 1))
        .await
        .unwrap();
    (sim, host)
}

/// Two concurrent first frames to one peer create one participant
/// record: one creates it, the other awaits it and uses it.
#[tokio::test]
async fn concurrent_first_sends_create_one_record() {
    let dir = tempfile::tempdir().unwrap();
    let peer = hex::encode(rekindle_crypto::Identity::generate().public_key_bytes());
    let (_sim, host) = host_on_sim(&dir, &peer).await;
    let friends = HostFriends::of(&host.ctx);
    assert!(friends
        .held_participant_writer(&peer)
        .await
        .unwrap()
        .is_none());
    let (a, b) = tokio::join!(writer(&friends, &peer), writer(&friends, &peer));
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.record_key, b.record_key);
    assert_eq!(usize::from(a.created) + usize::from(b.created), 1);
    let held = friends
        .held_participant_writer(&peer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.record_key, a.record_key);
    assert!(!held.created);
}

/// The record is made without the network (our DHT offline), and is on
/// the network once published, before its key is sent: its `HEAD` holds
/// the empty head. Veilid creates a record locally and writes nothing
/// (0.5.8 `create_record.rs`), so a peer opening a participant record
/// nobody wrote got `KeyNotFound` ("participant record not opened;
/// retried").
#[tokio::test]
async fn a_record_made_offline_is_on_the_network_before_its_key_is_sent() {
    use rekindle_types::dht_layout::dm_participant;
    let dir = tempfile::tempdir().unwrap();
    let peer = hex::encode(rekindle_crypto::Identity::generate().public_key_bytes());
    let (sim, host) = host_on_sim(&dir, &peer).await;
    let friends = HostFriends::of(&host.ctx);
    sim.fault(rekindle_netsim::Fault::DhtOffline {
        node: 0,
        offline: true,
    });
    let created = writer(&friends, &peer).await.unwrap();
    assert!(created.created);
    assert!(
        rekindle_transport::operations::dm::stream::publish(&friends, &peer, &created.record_key)
            .await
            .is_err(),
        "the key is not sent while the record cannot land"
    );
    sim.fault(rekindle_netsim::Fault::DhtOffline {
        node: 0,
        offline: false,
    });
    rekindle_transport::operations::dm::stream::publish(&friends, &peer, &created.record_key)
        .await
        .expect("the record lands");
    let head = sim
        .network_get(&created.record_key, dm_participant::HEAD)
        .expect("the participant record is on the network");
    assert!(
        rekindle_codec::capnp_codec::chain::decode_chain_head(&head.data)
            .unwrap()
            .is_none(),
        "its HEAD is the empty head"
    );
}
