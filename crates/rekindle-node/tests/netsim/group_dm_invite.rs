//! Growing a room (plan E7.2 G8, Xfire's "Invite users to room"): the
//! creator of a room of three invites a fourth friend, who accepts on the
//! E2.6 path; every member, old and new, then reads every other's message.
//! Only the creator invites (`inviteRefused` says so to the others), and a
//! room never passes eight members.

use rekindle_ipc::protocol::IpcRequest;

use crate::group_dm::{reads, send, three_members, view};
use crate::harness::{eventually, start};
use crate::pair::{befriend, key, LIMIT};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_creator_grows_a_room_of_three_to_four_and_never_past_eight() {
    let (sim, mut nodes) = start(4, 127).await;
    let gid = three_members(&sim, &mut nodes[..3]).await;
    nodes[3].unlock_new_identity("Dan").await;
    befriend(&sim, &nodes[3], &nodes[0]).await;

    let ann = view(&nodes[0], &gid).await.expect("Ann's room");
    assert_eq!(ann.invite_refused, None, "the creator may invite");
    let bea = view(&nodes[1], &gid).await.expect("Bea's room");
    assert!(bea.invite_refused.is_some_and(|r| r.contains("creator")));
    let refused = nodes[1]
        .call(IpcRequest::GroupDmInvite {
            group_id: gid.clone(),
            members: vec![key(&nodes[3])],
        })
        .await;
    match refused {
        Ok(_) => panic!("Bea grew a room she did not create"),
        Err(r) => assert!(r.message.contains("creator"), "{}", r.message),
    }

    nodes[0]
        .ok(IpcRequest::GroupDmInvite {
            group_id: gid.clone(),
            members: vec![key(&nodes[3])],
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async {
            view(&nodes[3], &gid)
                .await
                .is_some_and(|v| v.pending && v.members.len() == 4)
        })
        .await,
        "Dan holds the invite to the room of four"
    );
    nodes[3]
        .ok(IpcRequest::GroupDmAccept {
            group_id: gid.clone(),
        })
        .await;
    for node in &nodes {
        assert!(
            eventually(&sim, LIMIT, || async {
                view(node, &gid)
                    .await
                    .is_some_and(|v| !v.pending && v.members.len() == 4)
            })
            .await,
            "node {} holds the roster of four",
            node.index
        );
    }
    for (i, name) in ["Ann", "Bea", "Cat", "Dan"].iter().enumerate() {
        let body = format!("{name} says hi to four");
        send(&nodes[i], &gid, &body).await;
        for reader in &nodes {
            assert!(
                eventually(&sim, LIMIT * 2, || reads(reader, &gid, &body)).await,
                "node {} reads {body:?}",
                reader.index
            );
        }
    }

    // Five more would make nine.
    let five: Vec<String> = "56789".chars().map(|c| c.to_string().repeat(64)).collect();
    let full = nodes[0]
        .call(IpcRequest::GroupDmInvite {
            group_id: gid.clone(),
            members: five,
        })
        .await;
    match full {
        Ok(_) => panic!("the room passed eight members"),
        Err(r) => assert!(r.message.contains("at most 8"), "{}", r.message),
    }
    assert_eq!(
        view(&nodes[0], &gid).await.map(|v| v.members.len()),
        Some(4)
    );
}
