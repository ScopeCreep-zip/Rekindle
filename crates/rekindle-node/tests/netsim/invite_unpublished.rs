//! An invite link is handed out only after its `InviteCreated` op and our
//! frontier are on our member record `M` (plan E3.2 S6 (4)): a joiner's
//! walk reads what is on the DHT. While every write of the inviter's `M`
//! lands below consensus (`Fault::WriteFails`, as a Veilid `set_value`
//! can), the invite's secrets record still lands but its op does not, so
//! invite create waits through the outbox's retries up to the command's
//! deadline and is refused "not yet published", with no link. Once the
//! fault clears, a new invite lands and a joiner uses it.

use rekindle_ipc::protocol::LifecycleRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_netsim::Fault;
use rekindle_types::dht_layout::community_anchor;

use crate::harness::Community;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn no_invite_link_until_its_op_is_on_the_dht() {
    let mut community = Community::create(2, 223).await;
    let cert = community
        .sim
        .network_get(&community.governance_key, community_anchor::GENESIS_CERT)
        .expect("the anchor's GenesisCert");
    let creator_record = rekindle_codec::governance_op::genesis::decode_genesis(&cert.data)
        .expect("a GenesisCert")
        .signed
        .cert
        .creator_member_record;
    community.sim.fault(Fault::WriteFails {
        node: 0,
        record: creator_record,
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
    assert!(
        refused.is_err() && answer.contains("not yet published"),
        "no link while the op is off the DHT: {answer}"
    );

    community.sim.fault(Fault::Heal);
    // A fresh invite (its op lands behind the queued one) and a join by it.
    community.join(1).await;
}
