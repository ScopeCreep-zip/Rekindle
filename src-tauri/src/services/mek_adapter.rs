//! Phase 17 — MEK rotation adapter.
//!
//! Implements `rekindle_mek_rotation::MekDistributeDeps` against the
//! live AppState + AppHandle + Db. The crate's
//! `distribute_mek` / `wait_for_rotation_slot` flows parameterise
//! over this trait so the protocol logic stays free of Tauri/Veilid
//! concerns (Invariant 2).
//!
//! Phase 17.f.1 scaffolding — adapter struct + trait impl. Sub-step
//! 17.f.2 will thin `services/community/mek_rotation.rs` +
//! `mek_rotation_support.rs` to delegate via this adapter. Its cache is
//! `state_helpers::LiveMekCache` over `AppState.meks`.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_mek_rotation::{
    ChannelMekCache, MekDistributeDeps, MekPersist, MekRotationError, MekRotationEvent,
    RotationRecipient,
};
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::PseudonymKey;

use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// MEK rotation domain adapter — produced once per command/dispatch.
pub struct MekAdapter {
    state: Arc<AppState>,
    app_handle: tauri::AppHandle,
    _pool: Db,
    cache: Arc<dyn ChannelMekCache>,
    persist: Arc<dyn MekPersist>,
}

impl MekAdapter {
    #[must_use]
    pub fn new(state: Arc<AppState>, app_handle: tauri::AppHandle, pool: Db) -> Arc<Self> {
        let cache: Arc<dyn ChannelMekCache> =
            Arc::new(state_helpers::LiveMekCache::new(Arc::clone(&state)));
        let persist: Arc<dyn MekPersist> = Arc::new(
            rekindle_node::host::adapters::mek::VaultMekPersist::new(state.keystore.clone()),
        );
        Arc::new(Self {
            state,
            app_handle,
            _pool: pool,
            cache,
            persist,
        })
    }
}

#[async_trait]
impl MekDistributeDeps for MekAdapter {
    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope> {
        crate::state_helpers::login_scope_or_closed(&self.state)
    }

    fn rng(&self) -> std::sync::Arc<dyn rekindle_utils::RandomSource> {
        std::sync::Arc::clone(&self.state.rng)
    }

    fn cache(&self) -> Arc<dyn ChannelMekCache> {
        Arc::clone(&self.cache)
    }

    fn persist(&self) -> Arc<dyn MekPersist> {
        Arc::clone(&self.persist)
    }

    fn my_pseudonym(&self, community_id: &str) -> Option<PseudonymKey> {
        state_helpers::pseudonym_credentials(&self.state, community_id)
            .ok()
            .map(|(pseudo, _)| pseudo)
    }

    fn may_rotate(&self, community_id: &str, member: &PseudonymKey) -> bool {
        state_helpers::governance_state(&self.state, community_id)
            .is_some_and(|state| rekindle_governance::permissions::may_rotate_mek(member, &state))
    }

    fn online_recipients(
        &self,
        community_id: &str,
        exclude_pseudonym: Option<&str>,
    ) -> Vec<RotationRecipient> {
        let communities = self.state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return Vec::new();
        };
        let excluded = exclude_pseudonym.unwrap_or_default();
        community
            .gossip
            .as_ref()
            .map(|gossip| {
                gossip
                    .online_members
                    .iter()
                    .filter(|(pseudonym, member)| {
                        !member.route_blob.is_empty() && pseudonym.as_str() != excluded
                    })
                    .map(|(pseudonym, member)| RotationRecipient {
                        pseudonym_hex: pseudonym.clone(),
                        route_blob: member.route_blob.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn broadcast_to_peer(
        &self,
        _community_id: &str,
        peer_pseudonym_hex: &str,
        route_blob: &[u8],
        envelope_bytes: Vec<u8>,
    ) -> Result<Vec<u8>, MekRotationError> {
        state_helpers::call_route_blob(&self.state, route_blob, envelope_bytes)
            .await
            .map_err(|e| MekRotationError::Transport(format!("{peer_pseudonym_hex}: {e}")))
    }

    fn emit_event(&self, event: MekRotationEvent) {
        let mapped = match event {
            MekRotationEvent::RotationStarted {
                community_id,
                scope,
                new_generation,
                ..
            } => rekindle_types::subscription_events::CryptoEvent::MekRotated {
                community: community_id,
                channel: scope.wire_channel(),
                generation: new_generation,
                // The rotation-started signal names no rotator; the
                // peer-to-peer transfer that follows carries it.
                rotator_pseudonym: None,
            },
            MekRotationEvent::RotationComplete {
                community_id,
                scope,
                generation,
            } => rekindle_types::subscription_events::CryptoEvent::MekRotated {
                community: community_id,
                channel: scope.wire_channel(),
                generation,
                rotator_pseudonym: None,
            },
            // RotationFailed + MekDelivered have no current src-tauri
            // CommunityEvent counterpart — log instead. (Future:
            // surface as a NotificationEvent when the protocol grows
            // explicit failure UX.)
            MekRotationEvent::RotationFailed {
                community_id,
                scope,
                reason,
            } => {
                tracing::warn!(
                    community = %community_id,
                    %scope,
                    %reason,
                    "MEK rotation failed"
                );
                return;
            }
            // Was trace-only because the desktop's `CommunityEvent` had
            // no counterpart. Tier 1 has carried `MekTransferred` all
            // along, so the event is emitted now rather than dropped —
            // a CLI can show which peer supplied a key.
            MekRotationEvent::MekDelivered {
                community_id,
                scope,
                generation,
                sender_pseudonym_hex,
            } => rekindle_types::subscription_events::CryptoEvent::MekTransferred {
                community: community_id,
                channel: scope.wire_channel(),
                generation,
                sender_pseudonym: sender_pseudonym_hex,
            },
        };
        crate::event_dispatch::emit_subscription(
            &self.app_handle,
            &rekindle_types::subscription_events::SubscriptionEvent::Crypto(mapped),
        );
    }

    fn next_governance_lamport(
        &self,
        community_id: &str,
    ) -> Result<u64, rekindle_types::lamport::LamportError> {
        state_helpers::next_governance_lamport(&self.state, community_id)
    }

    fn identity_secret(&self) -> Option<[u8; 32]> {
        state_helpers::identity_secret(&self.state)
    }

    fn apply_received_mek_to_state(
        &self,
        community_id: &str,
        scope: KeyScope,
        mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
    ) -> bool {
        // Centralized downgrade-refuse + same-generation split-brain
        // resolution (lowest election rank wins). Refused → don't touch
        // generation state / media-ready either.
        if !state_helpers::install_mek(&self.state, community_id, scope, mek.clone()) {
            return false;
        }
        crate::services::community::mek_rotation_support::update_generation_state(
            &self.state,
            community_id,
            scope,
            mek.generation(),
        );
        true
    }

    fn persist_received_mek(
        &self,
        community_id: &str,
        scope: KeyScope,
        mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
    ) {
        crate::services::community::mek_rotation_support::persist_mek(
            &self.state,
            community_id,
            scope,
            mek,
        );
    }

    fn emit_rotation_received(&self, community_id: &str, scope: KeyScope, generation: u64) {
        crate::services::community::mek_rotation_support::emit_rotation_event(
            &self.app_handle,
            community_id,
            scope,
            generation,
        );
    }

    async fn write_governance_entry(
        &self,
        community_id: &str,
        entry: rekindle_types::governance::GovernanceEntry,
    ) -> Result<(), rekindle_mek_rotation::MekRotationError> {
        crate::services::community::write_entry(&self.state, community_id, entry)
            .await
            .map_err(rekindle_mek_rotation::MekRotationError::InvalidInput)
    }

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &rekindle_codec::community::envelope::CommunityEnvelope,
    ) -> Result<(), rekindle_mek_rotation::MekRotationError> {
        crate::services::community::send_to_mesh(&self.state, community_id, envelope)
            .map_err(rekindle_mek_rotation::MekRotationError::InvalidInput)
    }
}
