//! Per-phase "dial-in" gates for the self-sovereign join flow.
//!
//! The join is a sequence of network-bound phases (governance snapshot,
//! invite decode, slot claim, presence collection, record open, watch).
//! Historically the only bound was a single 15s timer on the *frontend*
//! `await`, which (a) was far too short for a multi-segment governance
//! scan plus a CAS slot claim with auto-expand, and (b) could not say
//! *which* phase hung — the backend kept running after the UI gave up,
//! so the community "appeared" later in a half-open state.
//!
//! [`gate`] gives each phase its own budget and emits a
//! [`GovernanceRuntimeEvent::JoinProgress`] before and after, so the
//! gating logic lives entirely in the backend and the UI is a pure display
//! of the progress stream. A phase past its budget surfaces a phase-named
//! error instead of a silent hang.
//!
//! The budget is a deadline the phase observes, not a timeout that drops
//! it: a dropped Veilid call is not cancelled (a slot-claim write could
//! land with nobody recording it), and Veilid logs a dropped API future as
//! an error. The deadline rides a task-local, so every checkpoint in the
//! phase — orchestrator or adapter — stops before its next Veilid call via
//! [`should_stop`] (plan C4.L1b).

use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;

use crate::deps::GovernanceRuntimeDeps;
use crate::event::{GovernanceRuntimeEvent, JoinStageStatus};

/// An ordered phase of the self-sovereign join flow. Each phase carries
/// its own timeout budget and a user-facing label for the dial-in UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinPhase {
    /// Multi-segment governance DHT scan + W26 signature verify + CRDT
    /// re-merge into the initial `CommunityState`.
    GovernanceSnapshot,
    /// Stronghold + invite-secrets DFLT fetch + decrypt, and the
    /// optional bootstrap-bundle `app_call` to the inviter.
    DecodeInvite,
    /// Registry slot CAS claim (up to 5 retries) with Plate-Gate
    /// auto-expand fallback (waits on an admin / self-expansion).
    ClaimSlot,
    /// Scan the registry for the initial presence/peer set.
    CollectPresence,
    /// Take the governance + registry (writable) + channel-log records
    /// leases and hand them to the host, which watches them
    /// (`GovernanceRuntimeDeps::community_records_ready`).
    OpenRecords,
}

impl JoinPhase {
    /// Per-phase timeout budget. Sized from the architecture's §6.2 join
    /// sequence: the slot claim is the slowest (CAS retries + up to a 30s
    /// segment auto-expand); the governance snapshot scans every occupied
    /// subkey across segments; opening and watching records each issue one
    /// DHT op per record (governance + registry + every channel log), so
    /// they scale with channel count and get more headroom than the
    /// single-round-trip presence scan and invite decode.
    #[must_use]
    pub fn budget(self) -> Duration {
        match self {
            Self::GovernanceSnapshot => Duration::from_secs(25),
            Self::DecodeInvite => Duration::from_secs(15),
            Self::ClaimSlot => Duration::from_secs(40),
            Self::CollectPresence => Duration::from_secs(10),
            // The opens and the watches that follow them, which were two
            // phases of 20 s and 12 s.
            Self::OpenRecords => Duration::from_secs(32),
        }
    }

    /// User-facing label for the dial-in stepper.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::GovernanceSnapshot => "Loading community",
            Self::DecodeInvite => "Decoding invite",
            Self::ClaimSlot => "Claiming your slot",
            Self::CollectPresence => "Finding members",
            Self::OpenRecords => "Connecting to records",
        }
    }
}

tokio::task_local! {
    /// The running join phase's deadline.
    static PHASE_UNTIL: Instant;
}

/// Whether the current join phase is past its budget. `false` outside a
/// join phase.
#[must_use]
pub fn phase_expired() -> bool {
    PHASE_UNTIL
        .try_with(|until| Instant::now() >= *until)
        .unwrap_or(false)
}

/// The checkpoint every multi-call governance loop runs before its next
/// Veilid call: the session is ending, or the join phase is out of budget.
#[must_use]
pub fn should_stop<D: GovernanceRuntimeDeps + ?Sized>(deps: &D) -> bool {
    deps.scope().is_closed() || phase_expired()
}

fn progress(
    community_id: &str,
    phase: JoinPhase,
    status: JoinStageStatus,
) -> GovernanceRuntimeEvent {
    GovernanceRuntimeEvent::JoinProgress {
        community_id: community_id.to_string(),
        stage_label: phase.label().to_string(),
        status,
    }
}

/// Run one join phase under its budget, emitting `JoinProgress` `Started`
/// before and `Done`/`Failed`/`TimedOut` after. The phase runs to its next
/// checkpoint past the deadline (never dropped mid-call) and a failure
/// after the deadline is reported as a phase-named timeout, so the caller
/// can abort the whole join with a precise message rather than hanging.
pub async fn gate<D, T, F>(
    deps: &D,
    community_id: &str,
    phase: JoinPhase,
    fut: F,
) -> Result<T, String>
where
    D: GovernanceRuntimeDeps,
    F: Future<Output = Result<T, String>>,
{
    deps.emit_event(progress(community_id, phase, JoinStageStatus::Started));
    let until = Instant::now() + phase.budget();
    match PHASE_UNTIL.scope(until, fut).await {
        Ok(value) => {
            deps.emit_event(progress(community_id, phase, JoinStageStatus::Done));
            Ok(value)
        }
        Err(_) if Instant::now() >= until => {
            deps.emit_event(progress(community_id, phase, JoinStageStatus::TimedOut));
            Err(format!(
                "{} timed out after {}s",
                phase.label(),
                phase.budget().as_secs()
            ))
        }
        Err(error) => {
            deps.emit_event(progress(community_id, phase, JoinStageStatus::Failed));
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
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
}
