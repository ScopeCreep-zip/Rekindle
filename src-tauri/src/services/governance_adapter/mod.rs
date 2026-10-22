//! Phase 18.h — governance runtime adapter.
//!
//! Implements `rekindle_governance_runtime::GovernanceRuntimeDeps`
//! against the live AppState + AppHandle + Db. The crate's
//! `apply::write_entry`, `origin::create_community`,
//! `bootstrap::build_bootstrap_response`, `segments::*`, and the join
//! primitives parameterise over this trait so the protocol logic stays
//! free of Tauri/Veilid concerns (Invariant 2).
//!
//! Phase 18.i — file split (Phase 14.r pattern):
//! - `mod.rs` (this file): struct + constructor + private parse helpers
//!   + free `snapshot_roles` / `snapshot_channels_and_categories` used
//!   by `emit_event`.
//! - `deps_impl.rs`: the `GovernanceRuntimeDeps` trait impl.

use std::sync::Arc;

use tauri::AppHandle;

use crate::state::AppState;
use rekindle_db::Db;
use rekindle_types::display::{CategoryDisplay, ChannelOverviewDisplay, RoleDisplay};

pub mod deps_impl;
mod dht;
mod events;
mod roles;
mod state_mutations;
mod state_reads;

/// Adapter — holds the three things every trait method needs: the
/// shared `AppState`, the Tauri `AppHandle` (for event emit + Db
/// lookup), and the `Db` clone (for bootstrap SQL queries).
pub struct GovernanceAdapter {
    pub(super) state: Arc<AppState>,
    pub(super) app_handle: AppHandle,
    pub(super) pool: Db,
}

impl GovernanceAdapter {
    pub fn new(state: Arc<AppState>, app_handle: AppHandle, pool: Db) -> Self {
        Self {
            state,
            app_handle,
            pool,
        }
    }
}

/// Phase 23.C entry point — chiral-split `open_community_dht_records`.
/// Mirrors the rebuild + hydrate entry-point shape.
pub async fn open_community_dht_records(state: &Arc<AppState>) {
    let Some(app_handle) = state.app_handle.read().clone() else {
        tracing::warn!("open_community_dht_records: app_handle not initialized");
        return;
    };
    let Ok(pool) = state.db.current() else {
        tracing::debug!("governance hydration: no identity database — dropped");
        return;
    };
    let adapter = GovernanceAdapter::new(Arc::clone(state), app_handle.clone(), pool.clone());
    rekindle_governance_runtime::dht_hydration::open_community_dht_records(&adapter).await;
}

/// Re-open our own locally-held write-once community records so veilid
/// rehydrates them — keeps invite-secrets DFLT records AND GovernanceOverflow
/// records alive across a restart (both have no retained owner keypair for live
/// writes). Runs after `rebuild_governance_from_dht` so `gov_state.invites` +
/// the overflow inventory are populated.
pub async fn republish_active_records(state: &Arc<AppState>) {
    let Some(app_handle) = state.app_handle.read().clone() else {
        tracing::warn!("republish_active_records: app_handle not initialized");
        return;
    };
    let Ok(pool) = state.db.current() else {
        tracing::debug!("governance hydration: no identity database — dropped");
        return;
    };
    let adapter = GovernanceAdapter::new(Arc::clone(state), app_handle.clone(), pool.clone());
    rekindle_governance_runtime::dht_hydration::republish_active_records(&adapter).await;
}

/// Phase 23.C entry point — chiral-split `hydrate_community_state_from_dht`.
/// Mirrors the rebuild entry-point shape.
pub async fn hydrate_community_state_from_dht(state: &Arc<AppState>) {
    let Some(app_handle) = state.app_handle.read().clone() else {
        tracing::warn!("hydrate_community_state_from_dht: app_handle not initialized");
        return;
    };
    let Ok(pool) = state.db.current() else {
        tracing::debug!("governance hydration: no identity database — dropped");
        return;
    };
    let adapter = GovernanceAdapter::new(Arc::clone(state), app_handle.clone(), pool.clone());
    rekindle_governance_runtime::dht_hydration::hydrate_community_state_from_dht(&adapter).await;
}

/// Phase 23.C entry point — chiral-split `rebuild_governance_from_dht`.
///
/// Constructs an adapter from the live AppState (+ AppHandle pulled
/// from `state.app_handle`) and delegates to the crate-side
/// orchestrator
/// `rekindle_governance_runtime::dht_hydration::rebuild_governance_from_dht`,
/// which is parameterised over `D: GovernanceRuntimeDeps`. All
/// CRDT-merge + sig-verify + lamport math is in the crate; this entry
/// point only assembles the deps and runs the orchestrator.
///
/// No-op when the AppHandle hasn't been registered yet (pre-`setup()`).
pub async fn rebuild_governance_from_dht(state: &Arc<AppState>) {
    let Some(app_handle) = state.app_handle.read().clone() else {
        tracing::warn!("rebuild_governance_from_dht: app_handle not initialized");
        return;
    };
    let Ok(pool) = state.db.current() else {
        tracing::debug!("governance hydration: no identity database — dropped");
        return;
    };
    let adapter = GovernanceAdapter::new(Arc::clone(state), app_handle.clone(), pool.clone());
    rekindle_governance_runtime::dht_hydration::rebuild_governance_from_dht(&adapter).await;
}

/// Rebuild one community's governance from its record (plan E3.1 C1): the
/// watch path passes the community whose governance record changed.
pub async fn rebuild_governance(state: &Arc<AppState>, community_id: &str) {
    let Some(app_handle) = state.app_handle.read().clone() else {
        return;
    };
    let Ok(pool) = state.db.current() else {
        return;
    };
    let adapter = GovernanceAdapter::new(Arc::clone(state), app_handle, pool);
    rekindle_governance_runtime::dht_hydration::rebuild_governance(&adapter, community_id).await;
}

// ---------- Free helpers used by `emit_event` ----------

pub(super) fn snapshot_roles(state: &Arc<AppState>, community_id: &str) -> Vec<RoleDisplay> {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .map(|community| {
            community
                .roles
                .iter()
                .map(|def| RoleDisplay {
                    id: def.id,
                    name: def.name.clone(),
                    color: def.color,
                    permissions: def.permissions,
                    position: def.position,
                    hoist: def.hoist,
                    mentionable: def.mentionable,
                    self_assignable: def.self_assignable,
                    exclusion_group: def.exclusion_group.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn snapshot_segments(
    state: &Arc<AppState>,
    community_id: &str,
) -> Vec<rekindle_types::presence::SegmentDescriptor> {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .map(|community| community.segments.clone())
        .unwrap_or_default()
}

pub(super) fn snapshot_channels_and_categories(
    state: &Arc<AppState>,
    community_id: &str,
) -> (Vec<ChannelOverviewDisplay>, Vec<CategoryDisplay>) {
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return (Vec::new(), Vec::new());
    };
    let channels = community
        .channels
        .iter()
        .enumerate()
        .map(|(index, channel)| ChannelOverviewDisplay {
            id: channel.id.clone(),
            name: channel.name.clone(),
            kind: channel.channel_type.to_string(),
            category_id: channel.category_id.clone(),
            topic: channel.topic.clone(),
            mek_generation: channel.mek_generation,
            log_key: channel.message_record_key.clone(),
            // `ChannelInfo` carries no ordinal of its own, but
            // `state_helpers::governance` sorts this Vec by the CRDT
            // `position` (then name) before storing it, so the index is
            // the display order rather than an arbitrary one.
            sort_order: u16::try_from(index).unwrap_or(u16::MAX),
            slowmode_seconds: channel.slowmode_seconds,
        })
        .collect();
    let categories = community
        .categories
        .iter()
        .map(|category| CategoryDisplay {
            id: category.id.clone(),
            name: category.name.clone(),
            sort_order: category.sort_order,
        })
        .collect();
    (channels, categories)
}
