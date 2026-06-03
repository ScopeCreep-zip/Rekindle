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
//! [`gate`] wraps each phase in its own [`tokio::time::timeout`] and
//! emits a [`GovernanceRuntimeEvent::JoinProgress`] before and after, so
//! the timeout/gating logic lives entirely in the backend and the UI is
//! a pure display of the progress stream. A stuck phase aborts at its
//! own budget and surfaces a phase-named error instead of a silent hang.

use std::future::Future;
use std::time::Duration;

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
    /// Open governance + registry (writable) + channel-log records and
    /// mark them tracked.
    OpenRecords,
    /// Establish DHT value watches on the opened records.
    WatchRecords,
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
            Self::OpenRecords => Duration::from_secs(20),
            Self::WatchRecords => Duration::from_secs(12),
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
            Self::WatchRecords => "Subscribing to updates",
        }
    }
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

/// Run one join phase under its own timeout, emitting `JoinProgress`
/// `Started` before and `Done`/`Failed`/`TimedOut` after. On timeout the
/// in-flight future is dropped (cancelled) and a phase-named error is
/// returned so the caller can abort the whole join with a precise
/// message rather than hanging.
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
    match tokio::time::timeout(phase.budget(), fut).await {
        Ok(Ok(value)) => {
            deps.emit_event(progress(community_id, phase, JoinStageStatus::Done));
            Ok(value)
        }
        Ok(Err(error)) => {
            deps.emit_event(progress(community_id, phase, JoinStageStatus::Failed));
            Err(error)
        }
        Err(_elapsed) => {
            deps.emit_event(progress(community_id, phase, JoinStageStatus::TimedOut));
            Err(format!(
                "{} timed out after {}s",
                phase.label(),
                phase.budget().as_secs()
            ))
        }
    }
}
