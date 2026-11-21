//! Domain events emitted by Phase 18 lifecycle ops.
//!
//! The src-tauri adapter maps these to existing `CommunityEvent` /
//! `NotificationEvent` variants on `app.emit("community-event", …)` and
//! `app.emit("notification-event", …)`. Defining them here keeps the
//! crate free of the Tauri channel types while preserving the existing
//! UI contract.

use rekindle_governance::merge::QuarantineReason;
use rekindle_types::governance::{GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;

/// Lifecycle status of a single join phase ("dial-in" gate). Carried by
/// [`GovernanceRuntimeEvent::JoinProgress`] so the UI can render each
/// phase as pending → active → done/failed/timed-out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinStageStatus {
    /// The phase has begun.
    Started,
    /// The phase finished successfully.
    Done,
    /// The phase returned an error before its deadline.
    Failed,
    /// The phase exceeded its per-phase timeout budget.
    TimedOut,
}

/// Lifecycle-side events. `RolesChanged` + `ChannelsUpdated` carry
/// **identifiers only** — the src-tauri adapter snapshots the current
/// in-memory `CommunityState` after applying and constructs the full
/// `CommunityEvent` payload (with `RoleDto` / `ChannelsUpdatedChannelDto`
/// / `ChannelsUpdatedCategoryDto`) from there. This keeps the crate
/// free of the src-tauri DTO types while preserving the existing UI
/// contract.
#[derive(Debug, Clone)]
pub enum GovernanceRuntimeEvent {
    /// A role-affecting governance entry has been applied — adapter
    /// snapshots roles and emits `CommunityEvent::RolesChanged`.
    RolesChanged { community_id: String },

    /// A channel/category-affecting governance entry has been applied —
    /// adapter snapshots channels/categories and emits
    /// `CommunityEvent::ChannelsUpdated`.
    ChannelsUpdated { community_id: String },

    /// A community has been created locally (origin flow,
    /// `seed_community`: every create, direct or co-created, lead
    /// decision (41)). The adapter publishes
    /// `GovernanceEvent::MetadataChanged`, the community list's arrival
    /// event.
    CommunityCreated { community_id: String, name: String },

    /// A held op's fate moved to quarantined in a merge (plan E3.2 S7c.5):
    /// the Host publishes `GovernanceEvent::GovernanceOpQuarantined`, once
    /// per op ([`crate::op_log::newly_quarantined`]).
    GovernanceOpQuarantined {
        community_id: String,
        op: OpHash,
        author: PseudonymKey,
        reason: QuarantineReason,
    },

    /// A held op is validly signed by a member but this reader cannot
    /// interpret it (D15): the community is quarantined with that reason;
    /// the Host publishes `GovernanceEvent::GovernanceOpQuarantined`
    /// (`UnknownOpKind`/`UnknownOpVersion`), once per op
    /// ([`crate::quarantine::newly_uninterpretable`]).
    CommunityQuarantined {
        community_id: String,
        op: OpHash,
        author: PseudonymKey,
        what: rekindle_codec::governance_op::Uninterpretable,
    },

    /// A community joined from a checkpoint hint finished its backfill and
    /// the full op set's state at the hint's cut misses the hinted
    /// snapshot's digest: a strictly endorsed checkpoint lied (plan E3.2
    /// S7c.8 (1), threat T8). The community stays on the backfilled state;
    /// the Host publishes `GovernanceEvent::CheckpointSnapshotMismatch`.
    CheckpointSnapshotMismatch {
        community_id: String,
        /// The hint's checkpoint height and digest.
        height: u64,
        digest: [u8; 32],
    },

    /// An applied successor both sides link (plan E3.2 S7s.3e part 2,
    /// [`crate::successor::announce`]): the Host publishes
    /// `GovernanceEvent::CommunitySuccessorOffered`, once per op per
    /// session.
    CommunitySuccessorOffered {
        community_id: String,
        new_anchor: String,
        signer: PseudonymKey,
        reason: rekindle_types::governance::SuccessorReason,
        creators: Vec<PseudonymKey>,
    },

    /// A bootstrap response has been built for a joiner. Adapter logs
    /// this for telemetry (no user-visible event today).
    BootstrapResponseBuilt {
        community_id: String,
        joiner_pseudonym_hex: String,
        bytes: usize,
    },

    /// A join attempt has progressed to a new stage. Adapter forwards
    /// to `CommunityEvent::JoinProgress { stage, status }`.
    JoinProgress {
        community_id: String,
        stage_label: String,
        status: JoinStageStatus,
    },

    /// A join has succeeded. Emitted by the Host's one join path
    /// (`HostGovernance::join`, a link's join and a co-creator's
    /// `CreatorJoin` alike, lead decision (41)); the adapter publishes
    /// `GovernanceEvent::MetadataChanged`, as for [`Self::CommunityCreated`].
    CommunityJoined { community_id: String, name: String },

    /// Plate Gate: a new segment has been added. Adapter logs.
    SegmentAdded {
        community_id: String,
        segment_index: u32,
    },

    /// Plate Gate: auto-expansion is pending (waiting for an admin to
    /// expand, or actively expanding). Adapter emits a
    /// `NotificationEvent::SystemAlert` so the user sees the 30-second
    /// wait isn't a hung join.
    JoinPendingAlert { have_manage_community: bool },

    /// A governance entry was just applied via `apply::write_entry` —
    /// adapter can use this to fan out auxiliary updates (e.g. lift the
    /// applied entry into telemetry). `GovernanceEntry` is boxed
    /// because the variant is ~400 B and would dominate the enum
    /// size otherwise (clippy::large_enum_variant).
    GovernanceEntryApplied {
        community_id: String,
        entry: Box<GovernanceEntry>,
    },

    /// A community's governance was rebuilt from its record (plan E3.1 C1)
    /// — the host publishes `GovernanceEvent::GovernanceRebuilt` and
    /// `MembershipEvent::MembersRefreshed` for it.
    GovernanceRebuilt { community_id: String },

    /// A member's role set changed — adapter emits
    /// `CommunityEvent::MemberRolesChanged`.
    MemberRolesChanged {
        community_id: String,
        pseudonym_hex: String,
        role_ids: Vec<rekindle_types::id::RoleId>,
    },

    /// A member completed onboarding — adapter emits
    /// `CommunityEvent::OnboardingComplete`.
    OnboardingComplete {
        community_id: String,
        pseudonym_hex: String,
        role_ids: Vec<rekindle_types::id::RoleId>,
    },
}
