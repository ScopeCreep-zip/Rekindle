//! Plan JP1: a governance rebuild never waits on the member card reads.
//! The inviter's bootstrap answer rebuilds inside the 4 s app_call
//! deadline (`op_log::receive_op`), and a card read is a DHT round trip
//! (a forced read of an unwritten subkey is a fanout to its timeout), so
//! the rebuild hands the reads to their own task and returns.

use rekindle_governance::state::{GovernanceState, MemberState};
use rekindle_types::governance::{AdmissionMode, GenesisCert};
use rekindle_types::id::PseudonymKey;

use crate::deps::MockGovernanceRuntimeDeps;

fn cert() -> GenesisCert {
    GenesisCert {
        name: "c".into(),
        policy: AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: PseudonymKey([9; 32]),
        creator_member_record: "VLD0:m".into(),
        creator_member_record_owner: "o".into(),
        created_at: 1,
        additional_creators: Vec::new(),
        predecessor: None,
        creator_device: None,
    }
}

/// A joined community "A" whose installed state holds one member whose
/// card name we lack: the card a refresh reads.
fn deps() -> MockGovernanceRuntimeDeps {
    let mut state = GovernanceState::default();
    state.members.insert(
        PseudonymKey([1; 32]),
        MemberState {
            member_record: "VLD0:joiner".into(),
            member_record_owner: "joiner-owner".into(),
            joined: rekindle_governance::op::OpStamp::MIN,
        },
    );
    let mut deps = MockGovernanceRuntimeDeps::new();
    deps.expect_list_community_governance_targets()
        .returning(|| vec![("A".into(), "VLD0:anchor".into())]);
    let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    deps.expect_rebuild_lock()
        .returning(move |_| std::sync::Arc::clone(&lock));
    let flight = std::sync::Arc::new(crate::rebuild_flight::RebuildFlight::default());
    deps.expect_rebuild_flight()
        .returning(move |_| std::sync::Arc::clone(&flight));
    let verdicts = std::sync::Arc::new(crate::verify_cache::OpVerifyCache::default());
    deps.expect_op_verify_cache()
        .returning(move |_| std::sync::Arc::clone(&verdicts));
    let judged = std::sync::Arc::new(crate::successor::SuccessorJudgements::default());
    deps.expect_successor_judgements()
        .returning(move |_| std::sync::Arc::clone(&judged));
    deps.expect_genesis_cert().returning(|_| Some(cert()));
    deps.expect_governance_state()
        .returning(move |_| Some(state.clone()));
    deps.expect_governance_ops().returning(|_| Ok(Vec::new()));
    deps.expect_governance_partial().returning(|_| Ok(None));
    deps.expect_set_governance_op_fates()
        .returning(|_, _| Ok(()));
    deps.expect_apply_governance_rebuild_result()
        .returning(|_, _| ());
    deps.expect_emit_event().returning(|_| ());
    deps.expect_record_governance_checkpoints()
        .returning(|_, _| Ok(()));
    deps.expect_warm_checkpoint_pages().returning(|_, _| ());
    deps.expect_set_governance_partial()
        .returning(|_, _| Ok(()));
    deps.expect_member_card_name().returning(|_, _| None);
    deps.expect_my_pseudonym().returning(|_| None);
    deps.expect_community_membership().returning(|_| None);
    deps
}

/// The rebuild installs, starts the card refresh as a task once, and
/// returns: it never opens the record port a card read goes through.
/// Fails without plan JP1 (the rebuild awaited `refresh_member_cards`,
/// whose first read reaches `record_port`).
#[tokio::test(start_paused = true)]
async fn a_rebuild_returns_without_waiting_for_the_card_reads() {
    let mut deps = deps();
    deps.expect_spawn_member_card_refresh()
        .withf(|community| community == "A")
        .times(1)
        .returning(|_| ());
    deps.expect_record_port().never();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        crate::rebuild_governance(&deps, "A"),
    )
    .await
    .expect("the rebuild returns at once");
}
