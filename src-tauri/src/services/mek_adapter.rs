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
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_mek_rotation::{
    ChannelMekCache, MekDistributeDeps, MekPersist, MekRotationError, MekRotationEvent,
    RotationRecipient,
};
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::{ChannelId, PseudonymKey};

use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Persists MEKs into the vault, per scope and generation
/// (`keystore::persist_mek` / `load_mek_generation`). `MekPersist`
/// takes/returns the raw 32-byte key material; we reconstruct
/// `MediaEncryptionKey` on either side.
pub struct KeystoreMekPersist {
    state: Arc<AppState>,
}

#[async_trait]
impl MekPersist for KeystoreMekPersist {
    async fn store_mek_for_generation(
        &self,
        community_id: &str,
        scope: KeyScope,
        generation: u64,
        key_bytes: Vec<u8>,
    ) -> Result<(), MekRotationError> {
        let key_bytes: [u8; 32] = key_bytes
            .try_into()
            .map_err(|_| MekRotationError::Persist("MEK bytes must be 32 long".into()))?;
        let mek = MediaEncryptionKey::from_bytes(key_bytes, generation);
        let guard = self.state.keystore.lock();
        let ks = guard
            .as_ref()
            .ok_or_else(|| MekRotationError::Persist("keystore locked".into()))?;
        crate::keystore::persist_mek(ks, community_id, scope, &mek)
            .map_err(MekRotationError::Persist)
    }

    async fn load_mek_for_generation(
        &self,
        community_id: &str,
        scope: KeyScope,
        generation: u64,
    ) -> Result<Option<Vec<u8>>, MekRotationError> {
        let guard = self.state.keystore.lock();
        let ks = guard
            .as_ref()
            .ok_or_else(|| MekRotationError::Persist("keystore locked".into()))?;
        Ok(
            crate::keystore::load_mek_generation(ks, community_id, scope, generation)
                .map(|m| m.as_bytes().to_vec()),
        )
    }
}

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
        let persist: Arc<dyn MekPersist> = Arc::new(KeystoreMekPersist {
            state: Arc::clone(&state),
        });
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

    async fn voice_recipients(
        &self,
        community_id: &str,
        channel: ChannelId,
        trigger_pseudonym: &str,
        include_trigger_in_recipients: bool,
    ) -> Vec<RotationRecipient> {
        let channel_id = channel.to_hex();
        use std::collections::HashSet;
        // voice rotation only targets peers currently in the voice
        // channel transport — plus the local member (if any) since
        // we're rotating for ourselves too. One awaited lock snapshots
        // keys + routes together; rotation always runs on the runtime,
        // so a blocking read here would panic the tokio worker (the
        // exact failure that killed join handling at app_state.rs:384).
        let (peer_keys, transport_routes): (Vec<String>, std::collections::HashMap<_, _>) =
            match self
                .state
                .voice_engine_transport_for_channel(community_id, &channel_id)
            {
                Some(transport) => {
                    let guard = transport.lock().await;
                    (
                        guard.peer_keys(),
                        guard.peer_entries().into_iter().collect(),
                    )
                }
                None => (Vec::new(), std::collections::HashMap::new()),
            };
        let mut participants = peer_keys.into_iter().collect::<HashSet<_>>();
        let my_pseudonym_hex = self.my_pseudonym(community_id).map(|p| hex::encode(p.0));
        if let Some(me) = my_pseudonym_hex {
            participants.insert(me);
        }
        if !include_trigger_in_recipients {
            participants.remove(trigger_pseudonym);
        }
        // Authoritative voice routes: each peer's route as advertised in
        // its VoiceJoin (already in the transport). Preferred over the
        // gossip presence overlay, which lags a fresh join — the cause
        // of the MEK being delivered to an empty route so a just-joined
        // peer never decrypts. online_members is the fallback.
        let communities = self.state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return Vec::new();
        };
        let gossip = community.gossip.as_ref();
        resolve_recipients(participants, &transport_routes, |pseudonym| {
            gossip
                .and_then(|g| g.online_members.get(pseudonym))
                .map(|member| member.route_blob.clone())
        })
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
        crate::services::community::media_ready_runtime::on_mek_updated(
            &self.state,
            community_id,
            scope.wire_channel().as_deref(),
        );
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

/// Resolve each participant's MEK-delivery route: the transport route
/// (advertised in the peer's VoiceJoin, authoritative for voice) is
/// preferred; `online_route` is the gossip-presence fallback for peers
/// not in the transport. A peer in neither gets an empty route. Pulled
/// out of `voice_recipients` so the transport-precedence rule (the fix
/// for the key being delivered to an empty route during the gossip lag)
/// is unit-testable.
fn resolve_recipients<F>(
    participants: impl IntoIterator<Item = String>,
    transport_routes: &std::collections::HashMap<String, Vec<u8>>,
    online_route: F,
) -> Vec<RotationRecipient>
where
    F: Fn(&str) -> Option<Vec<u8>>,
{
    participants
        .into_iter()
        .map(|pseudonym| {
            let route_blob = transport_routes
                .get(&pseudonym)
                .cloned()
                .or_else(|| online_route(&pseudonym))
                .unwrap_or_default();
            RotationRecipient {
                pseudonym_hex: pseudonym,
                route_blob,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::resolve_recipients;
    use std::collections::HashMap;

    #[test]
    fn transport_only_peer_gets_route_without_online_members() {
        // The bug: a just-joined peer is in the voice transport but not
        // yet in online_members, so the key went to an empty route. Now
        // the transport route is used, so the peer is reachable.
        let mut transport = HashMap::new();
        transport.insert("peerA".to_string(), vec![1, 2, 3]);
        let recipients = resolve_recipients(["peerA".to_string()], &transport, |_| None);
        assert_eq!(recipients.len(), 1);
        assert_eq!(recipients[0].pseudonym_hex, "peerA");
        assert_eq!(recipients[0].route_blob, vec![1, 2, 3]);
    }

    #[test]
    fn online_route_used_as_fallback_when_transport_missing() {
        let transport = HashMap::new();
        let recipients = resolve_recipients(["peerB".to_string()], &transport, |pk| {
            (pk == "peerB").then(|| vec![9, 9])
        });
        assert_eq!(recipients[0].route_blob, vec![9, 9]);
    }

    #[test]
    fn transport_route_preferred_over_online() {
        let mut transport = HashMap::new();
        transport.insert("peerC".to_string(), vec![1]);
        let recipients = resolve_recipients(["peerC".to_string()], &transport, |_| Some(vec![2]));
        assert_eq!(recipients[0].route_blob, vec![1]);
    }

    #[test]
    fn peer_in_neither_gets_empty_route() {
        let transport = HashMap::new();
        let recipients = resolve_recipients(["ghost".to_string()], &transport, |_| None);
        assert!(recipients[0].route_blob.is_empty());
    }
}
