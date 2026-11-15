//! Our own removal follows the merged state (plan E3.2 S9 own removal):
//! a removal the merge later voids (strong removal: concurrent with the
//! remover's own removal) clears the mark, so sends are not refused.

use std::sync::Arc;

use rekindle_governance::op::OpStamp;
use rekindle_governance::state::{GovernanceState, MemberState};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_types::id::PseudonymKey;

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::channel::HostChannel;
use crate::host::adapters::governance::HostGovernance;

/// Our pseudonym in `c1`, as a join stores it.
fn set_our_pseudonym(host: &crate::host::Host) -> PseudonymKey {
    let me = PseudonymKey([0xcd; 32]);
    if let Some(c1) = host.ctx.communities.map().write().get_mut("c1") {
        c1.my_pseudonym_key = Some(me.to_hex());
    }
    me
}

/// `me` removed by `by`; with `voided`, the merge kept `me` a member too
/// (the removal's author was removed concurrently, so its act is void).
fn removing(me: PseudonymKey, by: PseudonymKey, voided: bool) -> GovernanceState {
    let mut state = GovernanceState::default();
    state.removed.insert(me.clone(), OpStamp::MIN);
    state.removed_by.insert(me.clone(), by);
    if voided {
        state.members.insert(
            me,
            MemberState {
                member_record: "VLD0:member".into(),
                member_record_owner: String::new(),
                joined: OpStamp::MIN,
            },
        );
    }
    state
}

#[tokio::test]
async fn a_removal_the_merge_voids_stops_refusing_sends() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let (me, bea) = (set_our_pseudonym(&host), PseudonymKey([7; 32]));
    let governance = HostGovernance::new(Arc::clone(&host.ctx));
    let channel = HostChannel::new(&host.ctx);

    governance.set_governance_state("c1", removing(me.clone(), bea.clone(), false));
    let removal = host
        .ctx
        .communities
        .own_removal("c1")
        .expect("marked removed");
    assert_eq!(removal.by, Some(bea.clone()));
    assert!(
        channel.refuse_removed("c1").is_err(),
        "a removed member's send is refused"
    );

    governance.set_governance_state("c1", removing(me, bea, true));
    assert_eq!(
        host.ctx.communities.own_removal("c1"),
        None,
        "the void removal's mark goes"
    );
    assert!(
        channel.refuse_removed("c1").is_ok(),
        "the member sends again"
    );
    host.shutdown().await.unwrap();
}
