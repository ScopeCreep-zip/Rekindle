//! Group DM replies (owed item GD): Bea replies to Ann's message; the
//! parent's client id rides inside Bea's sealed entry and every member
//! (Ann, Bea, Cat) stores and serves the reply with it.

use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest};

use crate::group_dm::{history, three_members};
use crate::harness::{eventually, start, SimNode};
use crate::pair::LIMIT;

/// The parent `node` stores for its message `body`; `None` while absent.
async fn parent_of(node: &SimNode, gid: &str, body: &str) -> Option<Option<String>> {
    history(node, gid)
        .await
        .into_iter()
        .find(|m| m.body == body)
        .map(|m| m.reply_to_id)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_group_reply_reaches_every_member_with_its_parent() {
    let (sim, mut nodes) = start(3, 4101).await;
    let gid = three_members(&sim, &mut nodes).await;
    let parent = history(&nodes[0], &gid)
        .await
        .into_iter()
        .find(|m| m.body == "Ann says hi")
        .and_then(|m| m.message_id)
        .expect("Ann's message has a client id");
    nodes[1]
        .ok(IpcRequest::GroupDm(GroupDmRequest::Send {
            group_id: gid.clone(),
            body: "re: hi".to_string(),
            reply_to: Some(parent.clone()),
        }))
        .await;
    for node in &nodes {
        assert!(
            eventually(&sim, LIMIT * 2, || async {
                parent_of(node, &gid, "re: hi").await.is_some()
            })
            .await,
            "node {} reads the reply",
            node.index
        );
        assert_eq!(
            parent_of(node, &gid, "re: hi").await,
            Some(Some(parent.clone())),
            "node {} serves the reply's parent",
            node.index
        );
        assert_eq!(parent_of(node, &gid, "Ann says hi").await, Some(None));
    }
}
