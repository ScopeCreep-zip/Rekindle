use super::*;

fn peers(n: usize) -> Vec<PeerInfo> {
    (0..n)
        .map(|i| PeerInfo {
            pseudonym_key: format!("{i:064x}"),
            route_blob: vec![1],
        })
        .collect()
}

/// The pick keeps `gossip_degree(n)` and is the same for the same scope.
#[test]
fn the_pick_is_deterministic_and_degree_bound() {
    let a = pick(b"c\x01", peers(20));
    let b = pick(b"c\x01", peers(20).into_iter().rev().collect());
    assert_eq!(a.len(), 6);
    assert_eq!(
        a.iter().map(|p| &p.pseudonym_key).collect::<Vec<_>>(),
        b.iter().map(|p| &p.pseudonym_key).collect::<Vec<_>>()
    );
    assert_eq!(pick(b"c", peers(3)).len(), 3);
    assert_eq!(pick(b"c", peers(80)).len(), 8);
}

/// Two senders in one community do not pick the same peers: the scope
/// holds the sender, so every member of a large community is reached.
#[test]
fn senders_pick_different_peers() {
    let union: std::collections::HashSet<String> = (0u8..40)
        .flat_map(|sender| pick(&[b"c".as_slice(), &[sender]].concat(), peers(40)))
        .map(|p| p.pseudonym_key)
        .collect();
    assert!(union.len() > 30, "{}", union.len());
}

/// The broadcast held for want of peers in `c1`.
fn held() -> rekindle_codec::community::envelope::SignedEnvelope {
    rekindle_codec::community::envelope::SignedEnvelope {
        community_id: "c1".into(),
        sender_pseudonym: "00".repeat(32),
        message_id: [9; rekindle_codec::envelope::MESSAGE_ID_LEN],
        envelope_bytes: vec![1, 2, 3],
        signature: vec![0; 64],
        ttl: 5,
    }
}

/// How many broadcasts `c1` still holds.
fn still_held(host: &crate::host::Host) -> Option<usize> {
    let map = host.ctx.communities.map().read();
    map.get("c1")
        .and_then(|c| c.gossip.as_ref())
        .map(|g| g.pending_mesh_broadcasts.len())
}

/// A drain takes the held queue (plan E3.4 S5, JP1b): what it sent is
/// gone, so a second drain (the next beat's) finds nothing to resend.
#[tokio::test]
async fn a_drain_takes_the_held_queue() {
    use rekindle_governance::op::OpStamp;
    use rekindle_governance::state::{GovernanceState, MemberState};
    use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;

    use crate::host::runtime::tests::{create_ann, offline_host, seed_registry_community};

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let governance = HostGovernance::new(std::sync::Arc::clone(&host.ctx));
    let peer = PseudonymKey([0x42; 32]);
    let mut state = GovernanceState::default();
    state.members.insert(
        peer.clone(),
        MemberState {
            member_record: "VLD0:peer".into(),
            member_record_owner: String::new(),
            joined: OpStamp::MIN,
        },
    );
    governance.set_governance_state("c1", state);
    {
        let mut map = host.ctx.communities.map().write();
        let c1 = map.get_mut("c1").expect("seeded");
        c1.member_routes.observe(&peer, 1, vec![7]);
        let gossip = c1.gossip.get_or_insert_with(Default::default);
        gossip.pending_mesh_broadcasts.push_back(held());
        gossip.pending_mesh_broadcasts.push_back(held());
    }
    assert_eq!(governance.mesh_peers("c1").len(), 1, "one mesh peer");

    governance.drain_pending_mesh("c1");
    assert_eq!(still_held(&host), Some(0), "the drain took the queue");
    governance.drain_pending_mesh("c1");
    assert_eq!(still_held(&host), Some(0), "a second drain finds it empty");
    host.shutdown().await.unwrap();
}

/// Diag RC5 (plan E3.4 S5): a dead member route that clears, revived by
/// its probe or republished, is cached again and the broadcasts the mesh
/// held for want of peers go then, not only on the member's next beat
/// move; one still dead caches nothing and sends nothing.
#[tokio::test]
async fn a_cleared_dead_route_drains_the_held_broadcasts() {
    use rekindle_governance::op::OpStamp;
    use rekindle_governance::state::{GovernanceState, MemberState};
    use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
    use rekindle_route::dead_routes::Observed;

    use crate::host::runtime::tests::{create_ann, offline_host, seed_registry_community};

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let governance = HostGovernance::new(std::sync::Arc::clone(&host.ctx));
    let peer = PseudonymKey([0x42; 32]);
    let mut state = GovernanceState::default();
    state.members.insert(
        peer.clone(),
        MemberState {
            member_record: "VLD0:peer".into(),
            member_record_owner: String::new(),
            joined: OpStamp::MIN,
        },
    );
    governance.set_governance_state("c1", state);
    {
        let mut map = host.ctx.communities.map().write();
        let c1 = map.get_mut("c1").expect("seeded");
        let gossip = c1.gossip.get_or_insert_with(Default::default);
        gossip.pending_mesh_broadcasts.push_back(held());
        gossip.pending_mesh_broadcasts.push_back(held());
    }
    assert!(
        governance.mesh_peers("c1").is_empty(),
        "the dead route left the mesh empty"
    );

    governance.take_route("c1", &peer, 1, vec![7], Observed::Held, true);
    assert!(
        governance.mesh_peers("c1").is_empty(),
        "still dead: not cached"
    );
    assert_eq!(still_held(&host), Some(2), "still dead: nothing sent");

    governance.take_route("c1", &peer, 1, vec![7], Observed::Cleared, false);
    assert_eq!(
        governance.mesh_peers("c1").len(),
        1,
        "the cleared route is cached"
    );
    assert_eq!(
        still_held(&host),
        Some(0),
        "the clear drained the held broadcasts"
    );
    host.shutdown().await.unwrap();
}
