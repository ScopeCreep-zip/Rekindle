//! Phase 17 — MEK rotation adapter.
//!
//! Implements `rekindle_mek_rotation::MekDistributeDeps` against the
//! live AppState + AppHandle + DbPool. The crate's
//! `distribute_mek` / `wait_for_rotation_slot` flows parameterise
//! over this trait so the protocol logic stays free of Tauri/Veilid
//! concerns (Invariant 2).
//!
//! Phase 17.f.1 scaffolding — adapter struct + trait impl. Sub-step
//! 17.f.2 will thin `services/community/mek_rotation.rs` +
//! `mek_rotation_support.rs` to delegate via this adapter. AppState's
//! `mek_cache` + `channel_mek_cache` fields stay (48 non-rotation
//! callsites read them directly); the adapter's ChannelMekCache impl
//! wraps `state.channel_mek_cache` so both paths see the same data.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_mek_rotation::{
    ChannelMekCache, MekDistributeDeps, MekPersist, MekRotationError, MekRotationEvent,
    RotationRecipient,
};
use rekindle_types::id::PseudonymKey;

use crate::channels::CommunityEvent;
use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;

/// Wraps the existing `state.channel_mek_cache: Mutex<HashMap<(String,
/// String), MediaEncryptionKey>>` so the crate-side rotation flows
/// can read/write through the trait without us having to migrate the
/// 48 non-rotation callsites that touch the field directly.
pub struct AppStateMekCache {
    state: Arc<AppState>,
}

impl ChannelMekCache for AppStateMekCache {
    fn get(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Option<MediaEncryptionKey> {
        let cache = self.state.channel_mek_cache.lock();
        cache
            .get(&(community_id.to_string(), channel_id.to_string()))
            .filter(|mek| mek.generation() == generation)
            .cloned()
    }

    fn insert(&self, community_id: &str, channel_id: &str, mek: MediaEncryptionKey) {
        state_helpers::install_channel_mek(&self.state, community_id, channel_id, mek);
    }

    fn current_generation(&self, community_id: &str, channel_id: &str) -> u64 {
        self.state
            .channel_mek_cache
            .lock()
            .get(&(community_id.to_string(), channel_id.to_string()))
            .map_or(0, MediaEncryptionKey::generation)
    }
}

/// Persists MEKs into Stronghold via the existing
/// `keystore::store_mek` / `load_channel_mek_generation` helpers.
/// `MekPersist` takes/returns `Vec<u8>` (the raw 32-byte key
/// material); we reconstruct `MediaEncryptionKey` on either side.
pub struct KeystoreMekPersist {
    app_handle: tauri::AppHandle,
}

#[async_trait]
impl MekPersist for KeystoreMekPersist {
    async fn store_mek_for_generation(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
        wrapped_bytes: Vec<u8>,
    ) -> Result<(), MekRotationError> {
        let key_bytes: [u8; 32] = wrapped_bytes
            .try_into()
            .map_err(|_| MekRotationError::Persist("MEK bytes must be 32 long".into()))?;
        let mek = MediaEncryptionKey::from_bytes(key_bytes, generation);
        let keystore_handle =
            tauri::Manager::try_state::<crate::keystore::KeystoreHandle>(&self.app_handle)
                .ok_or_else(|| {
                    MekRotationError::Persist("keystore handle missing on app".into())
                })?;
        let guard = keystore_handle.inner().lock();
        if let Some(ks) = guard.as_ref() {
            crate::keystore::store_mek(ks, community_id, Some(channel_id), &mek);
        }
        Ok(())
    }

    async fn load_mek_for_generation(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Result<Option<Vec<u8>>, MekRotationError> {
        let keystore_handle =
            tauri::Manager::try_state::<crate::keystore::KeystoreHandle>(&self.app_handle)
                .ok_or_else(|| {
                    MekRotationError::Persist("keystore handle missing on app".into())
                })?;
        let guard = keystore_handle.inner().lock();
        let mek = guard.as_ref().and_then(|ks| {
            crate::keystore::load_channel_mek_generation(ks, community_id, channel_id, generation)
        });
        Ok(mek.map(|m| m.as_bytes().to_vec()))
    }
}

/// MEK rotation domain adapter — produced once per command/dispatch.
pub struct MekAdapter {
    state: Arc<AppState>,
    app_handle: tauri::AppHandle,
    _pool: DbPool,
    cache: Arc<dyn ChannelMekCache>,
    persist: Arc<dyn MekPersist>,
}

impl MekAdapter {
    #[must_use]
    pub fn new(state: Arc<AppState>, app_handle: tauri::AppHandle, pool: DbPool) -> Arc<Self> {
        let cache: Arc<dyn ChannelMekCache> = Arc::new(AppStateMekCache {
            state: Arc::clone(&state),
        });
        let persist: Arc<dyn MekPersist> = Arc::new(KeystoreMekPersist {
            app_handle: app_handle.clone(),
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
        channel_id: &str,
        trigger_pseudonym: &str,
        include_trigger_in_recipients: bool,
    ) -> Vec<RotationRecipient> {
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
                .voice_engine_transport_for_channel(community_id, channel_id)
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
        let api = state_helpers::veilid_api(&self.state)
            .ok_or_else(|| MekRotationError::Transport("Veilid API unavailable".into()))?;
        let route_id = api
            .import_remote_private_route(route_blob.to_vec())
            .map_err(|e| MekRotationError::Transport(format!("import route: {e}")))?;
        let rc = state_helpers::safe_routing_context(&self.state)
            .ok_or_else(|| MekRotationError::Transport("not attached".into()))?;
        let reply = rc
            .app_call(veilid_core::Target::RouteId(route_id), envelope_bytes)
            .await
            .map_err(|e| {
                MekRotationError::Transport(format!("app_call to {peer_pseudonym_hex}: {e}"))
            })?;
        Ok(reply)
    }

    fn emit_event(&self, event: MekRotationEvent) {
        let mapped = match event {
            MekRotationEvent::RotationStarted {
                community_id,
                channel_id,
                new_generation,
                ..
            } => CommunityEvent::MekRotated {
                community_id,
                channel_id: Some(channel_id),
                new_generation,
            },
            MekRotationEvent::RotationComplete {
                community_id,
                channel_id,
                generation,
            } => CommunityEvent::MekRotated {
                community_id,
                channel_id: Some(channel_id),
                new_generation: generation,
            },
            // RotationFailed + MekDelivered have no current src-tauri
            // CommunityEvent counterpart — log instead. (Future:
            // surface as a NotificationEvent when the protocol grows
            // explicit failure UX.)
            MekRotationEvent::RotationFailed {
                community_id,
                channel_id,
                reason,
            } => {
                tracing::warn!(
                    community = %community_id,
                    channel = %channel_id,
                    %reason,
                    "MEK rotation failed"
                );
                return;
            }
            MekRotationEvent::MekDelivered {
                community_id,
                channel_id,
                generation,
                sender_pseudonym_hex,
            } => {
                tracing::trace!(
                    community = %community_id,
                    channel = %channel_id,
                    generation,
                    sender = %sender_pseudonym_hex,
                    "MEK delivered from peer"
                );
                return;
            }
        };
        crate::event_dispatch::emit_live(&self.app_handle, "community-event", &mapped);
    }

    fn current_lamport(&self, community_id: &str) -> u64 {
        self.state
            .communities
            .read()
            .get(community_id)
            .map_or(0, |c| c.lamport_counter)
    }

    fn increment_lamport(&self, community_id: &str) -> u64 {
        state_helpers::increment_lamport(&self.state, community_id)
    }

    fn identity_secret(&self) -> Option<[u8; 32]> {
        *self.state.identity_secret.lock()
    }

    fn apply_received_mek_to_state(
        &self,
        community_id: &str,
        channel_id: Option<&str>,
        mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
    ) {
        let generation = mek.generation();
        match channel_id {
            Some(channel_id) if !channel_id.is_empty() => {
                if !state_helpers::install_channel_mek(
                    &self.state,
                    community_id,
                    channel_id,
                    mek.clone(),
                ) {
                    return;
                }
                crate::services::community::media_ready_runtime::on_mek_updated(
                    &self.state,
                    community_id,
                    Some(channel_id),
                );
                crate::services::community::mek_rotation_support::update_generation_state(
                    &self.state,
                    community_id,
                    Some(channel_id),
                    generation,
                );
            }
            _ => {
                {
                    let mut cache = self.state.mek_cache.lock();
                    if cache
                        .get(community_id)
                        .is_some_and(|cached| cached.generation() > generation)
                    {
                        tracing::debug!(
                            community = %community_id,
                            incoming = generation,
                            "community MEK transfer older than cached — not applied to live cache"
                        );
                        return;
                    }
                    cache.insert(community_id.to_string(), mek.clone());
                }
                crate::services::community::media_ready_runtime::on_mek_updated(
                    &self.state,
                    community_id,
                    None,
                );
                crate::services::community::mek_rotation_support::update_generation_state(
                    &self.state,
                    community_id,
                    None,
                    generation,
                );
            }
        }
    }

    fn persist_received_mek(
        &self,
        community_id: &str,
        channel_id: Option<&str>,
        mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
    ) {
        crate::services::community::mek_rotation_support::persist_mek(
            &self.app_handle,
            community_id,
            channel_id,
            mek,
        );
    }

    fn emit_rotation_received(
        &self,
        community_id: &str,
        channel_id: Option<&str>,
        generation: u64,
    ) {
        crate::services::community::mek_rotation_support::emit_rotation_event(
            &self.app_handle,
            community_id,
            channel_id,
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
        envelope: &rekindle_protocol::dht::community::envelope::CommunityEnvelope,
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
