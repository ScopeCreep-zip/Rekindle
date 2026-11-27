use super::*;
use crate::deps::MockGovernanceRuntimeDeps;

fn deps() -> MockGovernanceRuntimeDeps {
    let mut deps = MockGovernanceRuntimeDeps::new();
    deps.expect_emit_event().returning(|_| ());
    deps
}

/// A phase that stops at its own checkpoint once the budget is spent is
/// reported as a phase-named timeout, not dropped mid-call.
#[tokio::test(start_paused = true)]
async fn a_phase_past_its_budget_stops_at_its_checkpoint() {
    let deps = deps();
    let result: Result<(), String> = gate(&deps, "c1", JoinPhase::CollectPresence, async {
        while !phase_expired() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("stopped at checkpoint".into())
    })
    .await;
    assert_eq!(result, Err("Finding members timed out after 10s".into()));
    assert!(!phase_expired(), "no phase deadline outside a gate");
}

#[tokio::test(start_paused = true)]
async fn a_phase_within_its_budget_completes() {
    let deps = deps();
    let result = gate(&deps, "c1", JoinPhase::OpenRecords, async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!phase_expired());
        Ok::<_, String>(7)
    })
    .await;
    assert_eq!(result, Ok(7));
}
