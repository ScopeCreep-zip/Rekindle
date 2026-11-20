//! Plan E3.4 S5: the gossip mesh is the Host's, on both modes: the
//! governance roster's members whose beat is live, each reached by the
//! route its `M`/6 published (the member route cache), never by the
//! registry scan's overlay the presence poll rebuilt. A post's gossip
//! reaches every member over those routes.

use std::time::Duration;

use rekindle_records::port::RouteClass;

use crate::harness::Community;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_mesh_reaches_members_by_their_member_routes() {
    let community = Community::of(3, 5351).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let mesh = community.nodes[0]
        .host
        .governance()
        .mesh_peers(&community.anchor);
    for x in 1..community.nodes.len() {
        let hex = community.pseudonym(x).await;
        let peer = mesh
            .iter()
            .find(|p| p.pseudonym_key == hex)
            .unwrap_or_else(|| panic!("node {x} is in the creator's mesh: {mesh:?}"));
        assert_eq!(
            Some(&peer.route_blob),
            community.nodes[x]
                .routes
                .own_blob(RouteClass::General)
                .as_ref(),
            "node {x} is reached by the route its M/6 published"
        );
    }
    assert!(community.post(0, "over the mesh").await.is_ok());
    for x in 1..community.nodes.len() {
        assert!(
            community
                .reads(x, "over the mesh", Duration::from_secs(10))
                .await,
            "node {x} reads the post"
        );
    }
}
