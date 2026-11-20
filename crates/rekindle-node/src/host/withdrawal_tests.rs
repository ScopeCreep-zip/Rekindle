//! A join the merged state never admitted (it lost a single-use race, or
//! the inviter refused it) withdraws on leave: nothing is authored (plan
//! SU1, the withdrawal decision; ADR 0011: membership is the merged
//! state's alone), decided in the same state the leave's op would be
//! built on.

use rekindle_governance_runtime::moderation::{leave, LeaveOutcome};
use rekindle_governance_runtime::op_log::verify_held;
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_secrets::derive::derive_community_pseudonym;
use rekindle_types::id::PseudonymKey;

use super::{create_ann, meta, offline_host, seed_governance, seed_registry_community};
use super::{signed_chain, CREATOR_RECORD};

#[tokio::test]
async fn a_never_admitted_joins_leave_authors_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let gov = host.governance();
    // Someone else created `c1`; our `MemberJoined` is in no merge.
    let creator = derive_community_pseudonym(&[9; 32], "c1");
    let ops = signed_chain("c1", &creator, CREATOR_RECORD, vec![meta("C")]);
    let cert = seed_governance(&host, &ann.public_key, "c1", &creator, &ops).await;
    let me = derive_community_pseudonym(&gov.identity_secret().unwrap(), "c1");
    let state = rekindle_governance::merge::merge(&cert, &verify_held("c1", &ops).ops);
    assert!(!state
        .members
        .contains_key(&PseudonymKey(me.verifying_key().to_bytes())));
    {
        let mut map = host.ctx.communities.map().write();
        let c1 = map.get_mut("c1").unwrap();
        c1.genesis_cert = Some(cert);
        c1.my_pseudonym_key = Some(hex::encode(me.verifying_key().to_bytes()));
    }
    gov.set_governance_state("c1", state.clone());

    assert_eq!(
        leave(&gov, "c1").await.expect("the leave answers"),
        LeaveOutcome::Withdrawn
    );
    assert_eq!(
        gov.governance_ops("c1").await.unwrap().len(),
        ops.len(),
        "no op of ours is stored"
    );
    assert_eq!(
        gov.governance_state("c1"),
        Some(state),
        "the merged state is untouched"
    );
    host.shutdown().await.unwrap();
}
