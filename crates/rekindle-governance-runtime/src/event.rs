//! Domain events emitted by Phase 18 lifecycle ops.
//!
//! The src-tauri adapter maps these to existing `CommunityEvent` /
//! `NotificationEvent` variants on `app.emit("community-event", …)` and
//! `app.emit("notification-event", …)`. Defining them here keeps the
//! crate free of the Tauri channel types while preserving the existing
//! UI contract.

use rekindle_types::governance::GovernanceEntry;

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

    /// A community has been created locally (origin flow). Adapter emits
    /// `CommunityEvent::CommunityCreated` and starts background services.
    CommunityCreated {
        community_id: String,
        name: String,
    },

    /// A bootstrap response has been built for a joiner. Adapter logs
    /// this for telemetry (no user-visible event today).
    BootstrapResponseBuilt {
        community_id: String,
        joiner_pseudonym_hex: String,
        bytes: usize,
    },

    /// A join attempt has progressed to a new stage. Adapter forwards
    /// to `CommunityEvent::JoinProgress { stage_label }`.
    JoinProgress {
        community_id: String,
        stage_label: String,
    },

    /// A join has succeeded — adapter emits
    /// `CommunityEvent::CommunityJoined`.
    CommunityJoined {
        community_id: String,
        name: String,
    },

    /// Plate Gate: a new segment has been added. Adapter logs.
    SegmentAdded {
        community_id: String,
        segment_index: u32,
    },

    /// Plate Gate: auto-expansion is pending (waiting for an admin to
    /// expand, or actively expanding). Adapter emits a
    /// `NotificationEvent::SystemAlert` so the user sees the 30-second
    /// wait isn't a hung join.
    JoinPendingAlert {
        have_manage_community: bool,
    },

    /// A governance entry was just applied via `apply::write_entry` —
    /// adapter can use this to fan out auxiliary updates (e.g. lift the
    /// applied entry into telemetry). `GovernanceEntry` is boxed
    /// because the variant is ~400 B and would dominate the enum
    /// size otherwise (clippy::large_enum_variant).
    GovernanceEntryApplied {
        community_id: String,
        entry: Box<GovernanceEntry>,
    },
}
