//! Renaming a room (plan E7.2 G7, lead decision 1a): the creator's
//! `GroupDmRename` rides its group chain and the new name lands on every
//! member; another member's rename is refused and changes no one's name.
//! A malicious member skips our sender's creator check and commits a
//! sealed rename to its own group chain: every receiver refuses it
//! (`group_dm::rename::on_rename`, threat model: a malicious member).

use std::time::Duration;

use rekindle_codec::capnp_codec::direct::{DirectBody, GroupDmRename};
use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest};
use rekindle_transport::operations::dm::{client_id_text, outbox};
use rekindle_transport::operations::group_dm::parse_group_id;

use crate::group_dm::{three_members, view};
use crate::harness::{eventually, start};
use crate::pair::LIMIT;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_creator_renames_a_room_for_everyone_and_another_member_cannot() {
    let (sim, mut nodes) = start(3, 126).await;
    let gid = three_members(&sim, &mut nodes).await;
    nodes[0]
        .ok(IpcRequest::GroupDm(GroupDmRequest::Rename {
            group_id: gid.clone(),
            name: "scrim night".to_string(),
        }))
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
        .call(IpcRequest::GroupDm(GroupDmRequest::Rename {
            group_id: gid.clone(),
            name: "bea's room".to_string(),
        }))
        .await;
    match refused {
        Ok(_) => panic!("Bea renamed a room she did not create"),
        Err(r) => assert!(r.message.contains("creator"), "{}", r.message),
    }
    assert!(
        !eventually(&sim, Duration::from_secs(60), || async {
            for node in &nodes {
                if view(node, &gid)
                    .await
                    .is_some_and(|v| v.name != "scrim night")
                {
                    return true;
                }
            }
            false
        })
        .await,
        "no member's name changes"
    );
}

/// Bea, a member but not the creator, runs a sender with no creator check:
/// her `GroupDmRename` is sealed under her own group key and committed to
/// her own chain, exactly what our sender would commit for the creator.
/// Ann and Cat read her chain and refuse it; no one's name changes.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_s_forged_rename_is_refused_by_every_receiver() {
    let (sim, mut nodes) = start(3, 127).await;
    let gid = three_members(&sim, &mut nodes).await;
    let payload = outbox::payload(&DirectBody::GroupDmRename(GroupDmRename {
        group_id: parse_group_id(&gid).expect("a group id"),
        name: "bea's room".to_string(),
    }))
    .expect("a rename payload");
    let bea = nodes[1].host.friend_host();
    (bea.outbox().expect("Bea's outbox"))
        .enqueue_group_control(&gid, client_id_text(&[0xbe; 16]), payload)
        .await
        .expect("Bea's chain takes the entry");
    // Bea's next message rides the same chain, after the rename: once the
    // others hold it they have read past the rename.
    nodes[1]
        .ok(IpcRequest::GroupDm(GroupDmRequest::Send {
            group_id: gid.clone(),
            body: "after the rename".to_string(),
        }))
        .await;
    for node in [&nodes[0], &nodes[2]] {
        assert!(
            eventually(&sim, LIMIT, || async {
                crate::group_dm::history(node, &gid)
                    .await
                    .iter()
                    .any(|m| m.body == "after the rename")
            })
            .await,
            "node {} reads Bea's chain past the rename",
            node.index
        );
        let name = view(node, &gid).await.expect("the group").name;
        assert_eq!(
            name, "raid night",
            "node {} keeps the creator's name",
            node.index
        );
    }
}
