//! Our friend-list record lists a friend once they are accepted (live A/B
//! finding 50: only a removal pushed it, so it never held the friend and
//! the removal's push found it already empty).

use rekindle_codec::capnp_codec::friend::decode_friend_list;
use rekindle_ipc::protocol::IpcRequest;

use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, key, LIMIT};

fn lists(sim: &rekindle_netsim::Netsim, record: &str, peer: &str) -> bool {
    sim.network_get(record, 0)
        .and_then(|value| decode_friend_list(&value.data).ok())
        .is_some_and(|friends| friends.iter().any(|f| f.public_key == peer))
}

async fn friend_list_key(node: &SimNode) -> String {
    node.ok(IpcRequest::IdentityShow).await["friendListDhtKey"]
        .as_str()
        .expect("the friend list record is held")
        .to_owned()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn both_friend_list_records_list_the_friend_after_an_accept() {
    let (sim, mut nodes) = start(2, 103).await;
    for (node, name) in nodes.iter_mut().zip(["Kai", "Liv"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let (kai, liv) = (key(&nodes[0]), key(&nodes[1]));
    let (kai_list, liv_list) = (
        friend_list_key(&nodes[0]).await,
        friend_list_key(&nodes[1]).await,
    );
    assert!(
        eventually(&sim, LIMIT, || async { lists(&sim, &kai_list, &liv) }).await,
        "Kai's friend list record lists Liv"
    );
    assert!(
        eventually(&sim, LIMIT, || async { lists(&sim, &liv_list, &kai) }).await,
        "Liv's friend list record lists Kai"
    );
}
