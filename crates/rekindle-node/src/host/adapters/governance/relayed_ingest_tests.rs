//! Plan GI3: the Host's one ingest for relayed reactions and head hints
//! (both modes): the community gate and the gossip meter come before any
//! event or read.

use std::sync::Arc;
use std::time::Duration;

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_governance::state::MemberState;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_types::id::PseudonymKey;

use crate::host::runtime::tests::channel_gap_tests::with_channel;
use crate::host::runtime::tests::{create_ann, offline_host, seed_registry_community};

fn bob() -> PseudonymKey {
    PseudonymKey([9; 32])
}

/// A Host holding `c1` with Bob a member (or a departed one, banned).
async fn host_with_bob(departed: bool) -> (tempfile::TempDir, Arc<crate::host::Host>) {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let mut state = with_channel();
    state.members.insert(
        bob(),
        MemberState {
            member_record: "record-bob".into(),
            member_record_owner: "owner".into(),
            joined: rekindle_governance::fixture::stamp(2),
        },
    );
    if departed {
        state.bans.insert(bob());
    }
    (host.ctx.communities).set_governance_state("c1", state);
    (dir, host)
}

/// The daemon's gossip meter over `c1`.
fn meter(host: &crate::host::Host) {
    let scope = GovernanceRuntimeDeps::scope(&host.governance());
    let sub = rekindle_transport::SubscriptionManager::new(
        Arc::clone(&host.ctx.session),
        scope.child("subscriptions"),
        Arc::clone(&host.ctx.clock),
    );
    (sub.meshes().write()).insert(
        "c1".into(),
        rekindle_transport::gossip::GossipMesh::new("c1".into()),
    );
    *host.ctx.subscriptions.write() = Some(sub);
}

fn admits_bob(host: &crate::host::Host) -> bool {
    (host.ctx.subscriptions.read().as_ref()).is_some_and(|sub| sub.admit("c1", &bob().to_hex(), 0))
}

fn reaction_by(reactor: &str) -> ControlPayload {
    ControlPayload::ReactionAdded {
        channel_id: "general".into(),
        message_id: "m1".into(),
        emoji: "+1".into(),
        reactor_pseudonym: reactor.into(),
    }
}

/// The reaction events the journal carries within 300 ms.
async fn reactions_published(events: &mut rekindle_events::EventStream) -> usize {
    let mut published = 0;
    while let Ok(Some(frame)) =
        tokio::time::timeout(Duration::from_millis(300), events.next()).await
    {
        if format!("{frame:?}").contains("ReactionAdded") {
            published += 1;
        }
    }
    published
}

/// A member's own reaction becomes the Host's event.
#[tokio::test]
async fn a_members_reaction_is_published_by_the_host() {
    let (_dir, host) = host_with_bob(false).await;
    let mut events = host.ctx.journal.subscribe(None);
    let sender = bob().to_hex();
    (host.governance()).ingest_reaction("c1", &sender, 0, &reaction_by(&sender));
    assert_eq!(reactions_published(&mut events).await, 1);
}

/// A departed member's relayed reaction publishes nothing.
#[tokio::test]
async fn a_departed_members_reaction_publishes_nothing() {
    let (_dir, host) = host_with_bob(true).await;
    let mut events = host.ctx.journal.subscribe(None);
    let sender = bob().to_hex();
    (host.governance()).ingest_reaction("c1", &sender, 0, &reaction_by(&sender));
    assert_eq!(reactions_published(&mut events).await, 0);
}

/// A member over the gossip meter publishes nothing.
#[tokio::test]
async fn a_reaction_over_the_meter_publishes_nothing() {
    let (_dir, host) = host_with_bob(false).await;
    meter(&host);
    while admits_bob(&host) {}
    let mut events = host.ctx.journal.subscribe(None);
    let sender = bob().to_hex();
    (host.governance()).ingest_reaction("c1", &sender, 0, &reaction_by(&sender));
    assert_eq!(reactions_published(&mut events).await, 0);
}

/// A head hint the community gate refuses (we were removed) changes
/// nothing: it is not relayed, and it never reaches the meter, so twenty of
/// them leave the sender's bucket as it was.
#[tokio::test]
async fn a_head_hint_the_gate_refuses_changes_nothing() {
    let (_dir, host) = host_with_bob(false).await;
    meter(&host);
    host.ctx.communities.note_own_removal(
        "c1",
        crate::host::community::OwnRemoval {
            banned: false,
            by: None,
            by_name: None,
            reason: None,
        },
    );
    let sender = bob().to_hex();
    for _ in 0..20 {
        assert!(!(host.governance()).ingest_head_advanced("c1", &sender, b"hint"));
    }
    assert!(admits_bob(&host), "a refused hint consumed the meter");
}
