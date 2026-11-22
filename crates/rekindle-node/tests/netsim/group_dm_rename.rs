//! Renaming a room (plan E7.2 G7, lead decision 1a): the creator's
//! `GroupDmRename` rides its group chain and the new name lands on every
//! member; another member's rename is refused and changes no one's name.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;

use crate::group_dm::{three_members, view};
use crate::harness::{eventually, start};
use crate::pair::LIMIT;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_creator_renames_a_room_for_everyone_and_another_member_cannot() {
    let (sim, mut nodes) = start(3, 126).await;
    let gid = three_members(&sim, &mut nodes).await;
    nodes[0]
        .ok(IpcRequest::GroupDmRename {
            group_id: gid.clone(),
            name: "scrim night".to_string(),
        })
        .await;
    for node in &nodes {
        assert!(
            eventually(&sim, LIMIT, || async {
                view(node, &gid)
                    .await
                    .is_some_and(|v| v.name == "scrim night")
            })
            .await,
            "node {} reads the creator's name",
            node.index
        );
    }

    let refused = nodes[1]
        .call(IpcRequest::GroupDmRename {
            group_id: gid.clone(),
            name: "bea's room".to_string(),
        })
        .await;
    match refused {
        Ok(_) => panic!("Bea renamed a room she did not create"),
        Err(r) => assert!(r.message.contains("creator"), "{}", r.message),
    }
    assert!(
        !eventually(&sim, Duration::from_secs(60), || async {
            for node in &nodes {
                if view(node, &gid).await.is_some_and(|v| v.name != "scrim night") {
                    return true;
                }
            }
            false
        })
        .await,
        "no member's name changes"
    );
}
