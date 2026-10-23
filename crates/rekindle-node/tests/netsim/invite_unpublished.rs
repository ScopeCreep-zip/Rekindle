//! An invite link is handed out only after its `InviteCreated` op and our
//! frontier are on our member record `M` (plan E3.2 S6 (4)): a joiner's
//! walk reads what is on the DHT. With the inviter's DHT offline nothing
//! of the invite lands, so invite create is refused, with no link. Once the DHT is back, a new invite lands and a joiner uses it.

use rekindle_ipc::protocol::LifecycleRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_netsim::Fault;

use crate::harness::Community;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn no_invite_link_until_its_op_is_on_the_dht() {
    let mut community = Community::create(2, 223).await;
    community.sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });
    let refused = community.nodes[0]
        .call(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::InviteCreate {
                community: community.governance_key.clone(),
                max_uses: 1,
                expires_seconds: None,
            },
        )))
        .await;
    let answer = format!("{refused:?}");
    // Offline, the invite's secrets record is refused before its op is
    // queued; an op that is queued but not landed by the command's
    // deadline is refused "not yet published". Either way: no link.
    assert!(
        refused.is_err() && !answer.contains("rekindle://"),
        "no link while the DHT is offline: {answer}"
    );

    community.sim.fault(Fault::DhtOffline {
        node: 0,
        offline: false,
    });
    // A fresh invite (its op lands behind the queued one) and a join by it.
    community.join(1).await;
}
