//! Plan RV2d: the D16 key transfer to a joiner has a durable copy. The
//! inviter writes the sealed `MekTransfer` to its member record's key drop
//! (`M`/7) as well as calling it; the joiner watches that subkey of the
//! record its invite names until it holds a key. While every call to the
//! joiner hangs (`Fault::CallHangs`), the drop alone delivers its key; with
//! both paths up the joiner holds exactly one key.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_netsim::Fault;

use crate::harness::{eventually, Community};

fn held_keys(held: &serde_json::Value) -> usize {
    held.as_array()
        .map(Vec::len)
        .or_else(|| held.as_object().map(serde_json::Map::len))
        .unwrap_or(0)
}

async fn join(community: &mut Community) {
    let link = community.invite().await;
    community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityJoin { invite: link },
        )))
        .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_joiner_takes_its_key_from_the_drop_while_every_call_to_it_hangs() {
    let mut community = Community::create(2, 211).await;
    community.sim.fault(Fault::CallHangs {
        node: 1,
        hangs: true,
    });
    join(&mut community).await;
    assert!(
        eventually(&community.sim, Duration::from_secs(120), || async {
            held_keys(&community.mek_list(1).await) > 0
        })
        .await,
        "the key drop delivers the joiner's first key with no call reaching it"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn with_the_call_and_the_drop_both_up_the_joiner_holds_one_key() {
    let mut community = Community::create(2, 212).await;
    join(&mut community).await;
    assert!(
        eventually(&community.sim, Duration::from_secs(120), || async {
            held_keys(&community.mek_list(1).await) > 0
        })
        .await,
        "the joiner holds its first key"
    );
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(
        held_keys(&community.mek_list(1).await),
        1,
        "the second copy installs nothing new"
    );
}
