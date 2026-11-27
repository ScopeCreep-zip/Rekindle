use rekindle_records::port::RecordSchema;

use crate::Netsim;

/// A restored local store holds the value the node stored before the copy
/// was taken, not the newer one; the network keeps the newer replica.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_restored_local_store_rolls_back_the_node_s_copies() {
    let (sim, nodes) = Netsim::nodes(1, 1);
    let pool = nodes[0].ports.records.start_session().unwrap();
    let (lease, key, _) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    pool.set(lease, 0, b"old".to_vec(), None).await.unwrap();
    let backup = sim.local_store(0);
    pool.set(lease, 0, b"new".to_vec(), None).await.unwrap();
    sim.restore_local_store(0, backup);
    let local = pool.get(lease, 0, false).await.unwrap().unwrap();
    assert_eq!(local.data.as_slice(), b"old");
    assert_eq!(sim.network_get(&key, 0).unwrap().data.as_slice(), b"new");
}
