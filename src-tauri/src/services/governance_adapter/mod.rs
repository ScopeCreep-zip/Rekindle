//! Phase 18.h — governance runtime adapter.
//!
//! Implements `rekindle_governance_runtime::GovernanceRuntimeDeps`
//! against the live AppState + AppHandle + DbPool. The crate's
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
//! - `state_builder.rs`: the heaviest single method — `insert_community`
//!   body, which builds a fresh `CommunityState` from `CommunityInsert`.

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::channels::community_channel::{
    ChannelsUpdatedCategoryDto, ChannelsUpdatedChannelDto, RoleDto,
};
use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;

pub mod deps_impl;
mod dht;
mod events;
mod membership_events;
mod roles;
mod state_builder;
mod state_mutations;
mod state_reads;

pub(super) const RECENT_MESSAGES_LIMIT: i64 = 50;

/// Adapter — holds the three things every trait method needs: the
/// shared `AppState`, the Tauri `AppHandle` (for event emit + DbPool
/// lookup), and the `DbPool` clone (for bootstrap SQL queries).
pub struct GovernanceAdapter {
    pub(super) state: Arc<AppState>,
    pub(super) app_handle: AppHandle,
    pub(super) pool: DbPool,
}

impl GovernanceAdapter {
    pub fn new(state: Arc<AppState>, app_handle: AppHandle, pool: DbPool) -> Self {
        Self {
            state,
            app_handle,
            pool,
        }
    }

    pub(super) fn rc(
        &self,
    ) -> Result<veilid_core::RoutingContext, rekindle_governance_runtime::GovernanceRuntimeError>
    {
        state_helpers::safe_routing_context(&self.state)
            .ok_or(rekindle_governance_runtime::GovernanceRuntimeError::NotAttached)
    }

    pub(super) fn parse_writer_keypair(
        s: &str,
    ) -> Result<veilid_core::KeyPair, rekindle_governance_runtime::GovernanceRuntimeError> {
        s.parse::<veilid_core::KeyPair>().map_err(|e| {
            rekindle_governance_runtime::GovernanceRuntimeError::Adapter(format!(
                "invalid writer keypair: {e}"
            ))
        })
    }

    pub(super) fn parse_record_key(
        s: &str,
    ) -> Result<veilid_core::RecordKey, rekindle_governance_runtime::GovernanceRuntimeError> {
        s.parse::<veilid_core::RecordKey>().map_err(|e| {
            rekindle_governance_runtime::GovernanceRuntimeError::Adapter(format!(
                "invalid record key: {e}"
            ))
        })
    }
}

/// Phase 23.C entry point — chiral-split `open_community_dht_records`.
/// Mirrors the rebuild + hydrate entry-point shape.
pub async fn open_community_dht_records(state: &Arc<AppState>) {
    let Some(app_handle) = state.app_handle.read().clone() else {
        tracing::warn!("open_community_dht_records: app_handle not initialized");
        return;
    };
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let adapter =
        GovernanceAdapter::new(Arc::clone(state), app_handle.clone(), pool.inner().clone());
    rekindle_governance_runtime::dht_hydration::open_community_dht_records(&adapter).await;
}

/// Phase 23.C entry point — chiral-split `hydrate_community_state_from_dht`.
/// Mirrors the rebuild entry-point shape.
pub async fn hydrate_community_state_from_dht(state: &Arc<AppState>) {
    let Some(app_handle) = state.app_handle.read().clone() else {
        tracing::warn!("hydrate_community_state_from_dht: app_handle not initialized");
        return;
    };
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let adapter =
        GovernanceAdapter::new(Arc::clone(state), app_handle.clone(), pool.inner().clone());
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
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let adapter =
        GovernanceAdapter::new(Arc::clone(state), app_handle.clone(), pool.inner().clone());
    rekindle_governance_runtime::dht_hydration::rebuild_governance_from_dht(&adapter).await;
}

// ---------- Phase 23.E membership-event entry points ----------

/// Build an adapter from the live AppState + AppHandle. Shared by the
/// membership-event entry points below (all need the same three fields).
fn membership_adapter(state: &Arc<AppState>, app_handle: &AppHandle) -> GovernanceAdapter {
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    GovernanceAdapter::new(Arc::clone(state), app_handle.clone(), pool.inner().clone())
}

/// `ControlPayload::JoinAccepted` — cache MEK, persist members, derive
/// slot keypair, bootstrap peers.
pub async fn process_join_accepted(
    state: &Arc<AppState>,
    app_handle: &AppHandle,
    community_id: &str,
    sender_pseudonym: &str,
    input: rekindle_governance_runtime::membership_events::JoinAcceptedInput<'_>,
) {
    let adapter = membership_adapter(state, app_handle);
    rekindle_governance_runtime::membership_events::process_join_accepted(
        &adapter,
        community_id,
        sender_pseudonym,
        input,
    )
    .await;
}

/// `ControlPayload::MemberRolesChanged` — persist + emit.
pub fn process_member_roles_changed(
    state: &Arc<AppState>,
    app_handle: &AppHandle,
    community_id: &str,
    pseudonym_hex: &str,
    role_ids: &[u32],
) {
    let adapter = membership_adapter(state, app_handle);
    rekindle_governance_runtime::membership_events::process_member_roles_changed(
        &adapter,
        community_id,
        pseudonym_hex,
        role_ids,
    );
}

/// `ControlPayload::AdminKeypairGrant` — unwrap + install owner keypair.
pub fn process_admin_keypair_grant(
    state: &Arc<AppState>,
    app_handle: &AppHandle,
    community_id: &str,
    sender_pseudonym: &str,
    wrapped_owner_keypair: &[u8],
    wrapped_slot_seed: &[u8],
) {
    let adapter = membership_adapter(state, app_handle);
    rekindle_governance_runtime::membership_events::process_admin_keypair_grant(
        &adapter,
        community_id,
        sender_pseudonym,
        wrapped_owner_keypair,
        wrapped_slot_seed,
    );
}

/// `ControlPayload::SlotKeypairGrant` — unwrap + derive slot keypair.
pub fn process_slot_keypair_grant(
    state: &Arc<AppState>,
    app_handle: &AppHandle,
    community_id: &str,
    sender_pseudonym: &str,
    slot_index: u32,
    segment_index: u32,
    wrapped_slot_keypair: &[u8],
) {
    let adapter = membership_adapter(state, app_handle);
    rekindle_governance_runtime::membership_events::process_slot_keypair_grant(
        &adapter,
        community_id,
        sender_pseudonym,
        slot_index,
        segment_index,
        wrapped_slot_keypair,
    );
}

/// `ControlPayload::OnboardingAnswers` — validate answers, assign roles,
/// broadcast completion.
pub async fn process_onboarding_answers(
    state: &Arc<AppState>,
    app_handle: &AppHandle,
    community_id: &str,
    sender_pseudonym: &str,
    answers: &[rekindle_protocol::dht::community::envelope::OnboardingAnswer],
) {
    let adapter = membership_adapter(state, app_handle);
    rekindle_governance_runtime::membership_events::process_onboarding_answers(
        &adapter,
        community_id,
        sender_pseudonym,
        answers,
    )
    .await;
}

/// Peer-assisted join notice — note the peer + bump invite uses.
pub fn process_peer_assisted_join(
    state: &Arc<AppState>,
    app_handle: &AppHandle,
    community_id: &str,
    pseudonym_key: &str,
    display_name: &str,
    claimed_subkey_index: Option<u32>,
    route_blob: Option<&[u8]>,
    invite_code: Option<&str>,
) {
    let adapter = membership_adapter(state, app_handle);
    rekindle_governance_runtime::membership_events::process_peer_assisted_join(
        &adapter,
        community_id,
        pseudonym_key,
        display_name,
        claimed_subkey_index,
        route_blob,
        invite_code,
    );
}

/// Decrypt a channel message with the cached MEK (pure crate fn). Locks
/// `state.mek_cache`, snapshots the entry, and matches on generation.
#[must_use]
pub fn decrypt_channel_message(
    state: &Arc<AppState>,
    community_id: &str,
    ciphertext: &[u8],
    mek_generation: u64,
) -> rekindle_governance_runtime::membership_events::MekDecryptResult {
    let mek_cache = state.mek_cache.lock();
    rekindle_governance_runtime::membership_events::decrypt_with_cached_mek(
        &mek_cache,
        community_id,
        ciphertext,
        mek_generation,
    )
}

// ---------- Free helpers used by `emit_event` ----------

pub(super) fn snapshot_roles(state: &Arc<AppState>, community_id: &str) -> Vec<RoleDto> {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .map(|community| community.roles.iter().map(RoleDto::from).collect())
        .unwrap_or_default()
}

pub(super) fn snapshot_channels_and_categories(
    state: &Arc<AppState>,
    community_id: &str,
) -> (
    Vec<ChannelsUpdatedChannelDto>,
    Vec<ChannelsUpdatedCategoryDto>,
) {
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return (Vec::new(), Vec::new());
    };
    let channels = community
        .channels
        .iter()
        .map(|channel| ChannelsUpdatedChannelDto {
            id: channel.id.clone(),
            name: channel.name.clone(),
            channel_type: channel.channel_type.to_string(),
            category_id: channel.category_id.clone(),
            topic: channel.topic.clone(),
            slowmode_seconds: channel.slowmode_seconds,
        })
        .collect();
    let categories = community
        .categories
        .iter()
        .map(|category| ChannelsUpdatedCategoryDto {
            id: category.id.clone(),
            name: category.name.clone(),
            sort_order: category.sort_order,
        })
        .collect();
    (channels, categories)
}
