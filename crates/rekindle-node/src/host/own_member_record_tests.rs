//! Our own member record is the one the join or create recorded (plan
//! E3.2, ADR 0011), never read back from our merged roster: after our own
//! `MemberLeft` merges, the leave's frontier write and our outbox's head
//! still need it (netsim `left_slot_write`, LV1).

use std::sync::Arc;

use rekindle_governance::op::OpStamp;
use rekindle_governance::state::{GovernanceState, MemberState};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_types::id::PseudonymKey;

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::governance::HostGovernance;

const MINE: &str = "VLD0:mine";

/// Our merged state of `c1`: `me` a member on [`MINE`], or `me` departed
/// by our own `MemberLeft` (its op log kept, the member entry gone).
fn state(me: &PseudonymKey, left: bool) -> GovernanceState {
    let mut state = GovernanceState::default();
    if left {
        state.op_log_records.insert(me.clone(), MINE.into());
    } else {
        state.members.insert(
            me.clone(),
            MemberState {
                member_record: MINE.into(),
                member_record_owner: String::new(),
                joined: OpStamp::MIN,
            },
        );
    }
    state
}

#[tokio::test]
async fn our_own_record_resolves_after_our_own_member_left() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let me = PseudonymKey([0xcd; 32]);
    if let Some(c1) = host.ctx.communities.map().write().get_mut("c1") {
        c1.my_pseudonym_key = Some(me.to_hex());
    }
    host.ctx.communities.set_my_member_record("c1", MINE);
    let governance = HostGovernance::new(Arc::clone(&host.ctx));

    governance.set_governance_state("c1", state(&me, false));
    assert_eq!(governance.own_member_record("c1").as_deref(), Some(MINE));
    assert!(!governance.own_departed("c1"), "a member beats");

    governance.set_governance_state("c1", state(&me, true));
    assert_eq!(
        governance.own_member_record("c1").as_deref(),
        Some(MINE),
        "our own MemberLeft leaves our record ours to write"
    );
    assert!(governance.own_departed("c1"), "a leaver writes no beat");
    host.shutdown().await.unwrap();
}
