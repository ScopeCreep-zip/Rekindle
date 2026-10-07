//! Phase 13 — DM domain adapter.
//!
//! Implements `rekindle_dm::DmDeps` + `rekindle_dm::DmMekCache` against
//! the live `AppState`, `tauri::AppHandle`, `Db`, and Veilid
//! `RoutingContext`. Every veilid-core / Tauri / SQLite touch the DM
//! domain logic needs is realized here so `rekindle-dm` stays a
//! veilid-free, Tauri-free, AppState-free domain crate.
//!
//! Construct one `Arc<DmAdapter>` per session and hand it to
//! `rekindle_dm::send_dm_message`, `handle_dm_subkey_change`, etc. The
//! adapter cheaply clones the underlying `Arc<AppState>` + Db +
//! AppHandle handles.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_dm::{DmDeps, DmError, DmEvent, DmMekCache, DmMekChain, DmStore, SqliteDmStore};
use rekindle_protocol::dht::pool::RecordPool;
use rekindle_protocol::dht::schema;
use rekindle_protocol::messaging::envelope::MessagePayload;
use rekindle_records::lease::LeaseId;
use veilid_core::{BarePublicKey, BareSecretKey, KeyPair, PublicKey, RecordKey, CRYPTO_KIND_VLD0};

use crate::services::message_service;

use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// MEK cache that wraps `AppState.dm_mek_cache`. Holds a strong Arc to
/// AppState so the lock survives even if the adapter that constructed
/// it is dropped mid-operation (the lock is parking_lot, never crosses
/// `.await`).
struct AppStateMekCache {
    state: Arc<AppState>,
}

impl DmMekCache for AppStateMekCache {
    fn insert(&self, record_key: &str, chain: DmMekChain) {
        self.state
            .dm_mek_cache
            .lock()
            .insert(record_key.to_string(), chain);
    }

    fn current(&self, record_key: &str) -> Result<([u8; 32], u64), DmError> {
        let mut guard = self.state.dm_mek_cache.lock();
        let chain = guard
            .get_mut(record_key)
            .ok_or_else(|| DmError::MekChainUnavailable(record_key.to_string()))?;
        let (gen, mek) = chain
            .current()
            .map_err(|e| DmError::EncryptFailed(format!("chain current: {e}")))?;
        Ok((*mek.as_bytes(), gen))
    }

    fn observed_and_lookup(
        &self,
        record_key: &str,
        observed_gen: u64,
    ) -> Result<[u8; 32], DmError> {
        let mut guard = self.state.dm_mek_cache.lock();
        let chain = guard
            .get_mut(record_key)
            .ok_or_else(|| DmError::MekChainUnavailable(record_key.to_string()))?;
        chain
            .observed_generation(observed_gen)
            .map_err(|e| DmError::DecryptFailed(format!("chain materialize: {e}")))?;
        let mek = chain
            .for_generation(observed_gen)
            .map_err(|e| DmError::DecryptFailed(format!("chain lookup: {e}")))?;
        Ok(*mek.as_bytes())
    }

    fn advance(&self, record_key: &str) -> Result<u64, DmError> {
        let mut guard = self.state.dm_mek_cache.lock();
        let chain = guard
            .get_mut(record_key)
            .ok_or_else(|| DmError::MekChainUnavailable(record_key.to_string()))?;
        chain
            .advance()
            .map_err(|e| DmError::EncryptFailed(format!("chain advance: {e}")))
    }
}

/// DM domain adapter. Implements `DmDeps`; produced once per session.
pub struct DmAdapter {
    state: Arc<AppState>,
    app_handle: tauri::AppHandle,
    pool: Db,
    store: Arc<dyn DmStore>,
    mek_cache: Arc<dyn DmMekCache>,
}

impl DmAdapter {
    #[must_use]
    pub fn new(state: Arc<AppState>, app_handle: tauri::AppHandle, pool: Db) -> Arc<Self> {
        let store: Arc<dyn DmStore> = Arc::new(SqliteDmStore::new(pool.clone()));
        let mek_cache: Arc<dyn DmMekCache> = Arc::new(AppStateMekCache {
            state: Arc::clone(&state),
        });
        Arc::new(Self {
            state,
            app_handle,
            pool,
            store,
            mek_cache,
        })
    }
}

/// A slot writer from its Ed25519 `(secret, public)` bytes.
fn slot_keypair((secret, public): ([u8; 32], [u8; 32])) -> KeyPair {
    let veilid_pub = PublicKey::new(CRYPTO_KIND_VLD0, BarePublicKey::new(&public));
    KeyPair::new_from_parts(veilid_pub, BareSecretKey::new(&secret))
}

impl DmAdapter {
    fn record_pool(&self) -> Result<Arc<RecordPool>, DmError> {
        state_helpers::record_pool(&self.state).map_err(|_| DmError::RoutingContextUnavailable)
    }
}

#[async_trait]
impl DmDeps for DmAdapter {
    fn owner_key(&self) -> Result<String, DmError> {
        state_helpers::current_owner_key(&self.state).map_err(|_| DmError::IdentityNotLoaded)
    }

    fn identity_secret(&self) -> Result<[u8; 32], DmError> {
        state_helpers::identity_secret(&self.state).ok_or(DmError::IdentityNotLoaded)
    }

    fn store(&self) -> Arc<dyn DmStore> {
        Arc::clone(&self.store)
    }

    fn mek_cache(&self) -> Arc<dyn DmMekCache> {
        Arc::clone(&self.mek_cache)
    }

    async fn dht_create_smpl_record(
        &self,
        member_pubkeys: Vec<[u8; 32]>,
    ) -> Result<(LeaseId, String), DmError> {
        let smpl_schema = schema::community_smpl_schema(&member_pubkeys)
            .map_err(|e| DmError::transport(format!("dm smpl schema: {e}")))?;
        let (lease, key, _owner) = self
            .record_pool()?
            .create(smpl_schema, None)
            .await
            .map_err(|e| DmError::transport(format!("create dm dht record: {e}")))?;
        Ok((lease, key.to_string()))
    }

    async fn dht_acquire_record(
        &self,
        record_key: &str,
        writer_keypair: Option<([u8; 32], [u8; 32])>,
    ) -> Result<LeaseId, DmError> {
        let record_key_typed = record_key
            .parse::<RecordKey>()
            .map_err(|e| DmError::transport(format!("invalid record key: {e}")))?;
        self.record_pool()?
            .acquire(&record_key_typed, writer_keypair.map(slot_keypair))
            .await
            .map_err(|e| DmError::transport(format!("open dm record: {e}")))
    }

    async fn dht_release_record(&self, lease: LeaseId) {
        if let Ok(pool) = self.record_pool() {
            pool.release(lease).await;
        }
    }

    async fn dht_write_subkey(
        &self,
        lease: LeaseId,
        subkey: u32,
        value: Vec<u8>,
        writer_keypair: ([u8; 32], [u8; 32]),
    ) -> Result<(), DmError> {
        let outcome = self
            .record_pool()?
            .set(lease, subkey, value, Some(slot_keypair(writer_keypair)))
            .await
            .map_err(|e| DmError::transport(format!("write dm subkey: {e}")))?;
        if outcome.missed() {
            return Err(DmError::transport(format!(
                "write dm subkey: not stored ({outcome:?})"
            )));
        }
        Ok(())
    }

    async fn dht_watch_subkeys(&self, lease: LeaseId, subkeys: Vec<u32>) -> Result<(), DmError> {
        self.record_pool()?
            .watch(lease, subkeys.into_iter().collect())
            .await
            .map_err(|e| DmError::transport(format!("watch dm subkeys: {e}")))
    }

    async fn dht_hold_session(&self, record_key: &str, lease: LeaseId) {
        let surplus = {
            let mut held = self.state.dm_leases.lock();
            match held.entry(record_key.to_string()) {
                std::collections::hash_map::Entry::Occupied(_) => Some(lease),
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(lease);
                    None
                }
            }
        };
        if let Some(lease) = surplus {
            self.dht_release_record(lease).await;
        }
    }

    async fn dht_release_session(&self, record_key: &str) {
        let held = self.state.dm_leases.lock().remove(record_key);
        if let Some(lease) = held {
            self.dht_release_record(lease).await;
        }
    }

    async fn send_app_call(
        &self,
        peer_pubkey_hex: &str,
        payload: MessagePayload,
    ) -> Result<MessagePayload, DmError> {
        message_service::send_to_peer_call(&self.state, peer_pubkey_hex, &payload)
            .await
            .map_err(DmError::transport)
    }

    async fn send_encrypted(
        &self,
        peer_pubkey_hex: &str,
        payload: MessagePayload,
    ) -> Result<(), DmError> {
        message_service::send_to_peer(&self.state, &self.pool, peer_pubkey_hex, &payload)
            .await
            .map_err(DmError::transport)
    }

    fn emit_event(&self, event: DmEvent) {
        match event {
            DmEvent::MessageReceived {
                record_key,
                sender_pseudonym,
                body,
                timestamp_ms,
            } => {
                crate::event_dispatch::emit_subscription(
                    &self.app_handle,
                    &rekindle_types::subscription_events::SubscriptionEvent::ChannelMessage(rekindle_types::subscription_events::ChannelMessageEvent::DirectMessageReceived {
                        peer_key: sender_pseudonym,
                        body: Some(body),
                        decryption_failed: false,
                        automod_blurred: false,
                        timestamp: timestamp_ms,
                        conversation_id: record_key,
                        server_message_id: None,
                        reply_to_id: None,
                        sender_name: None,
                    }),
                );
            }
            DmEvent::InviteReceived {
                record_key,
                sender_pseudonym,
                sender_public_key_hex,
                is_group,
            } => {
                crate::event_dispatch::emit_subscription(
                    &self.app_handle,
                    &rekindle_types::subscription_events::SubscriptionEvent::ChannelMessage(rekindle_types::subscription_events::ChannelMessageEvent::DirectConversationInvited {
                        from: sender_public_key_hex,
                        record_key,
                        initiator_pseudonym: sender_pseudonym,
                        is_group,
                    }),
                );
            }
            DmEvent::InviteDeclined { record_key, reason } => {
                // No prior ChatEvent variant existed for inbound decline
                // (the old code just deleted the row silently); preserve
                // that by tracing only — the next conversation-list
                // refresh on the frontend picks up the row removal.
                tracing::info!(record_key, reason, "DM invite declined by peer");
            }
            DmEvent::GroupMemberLeft {
                record_key,
                sender_public_key_hex,
            } => {
                tracing::info!(
                    record_key,
                    peer = %sender_public_key_hex,
                    "DM group leave received"
                );
            }
            DmEvent::VideoFrameAssembled {
                sender_public_key_hex,
                stream_id,
                frame_seq,
                keyframe,
                codec,
                timestamp,
                data,
            } => {
                use base64::Engine as _;
                // Phase 11 Tier 1 — high-throughput frames bypass the
                // event bus and go to the per-peer `ipc::Channel` the DM
                // video panel registered. No-op when no panel is open.
                self.state.video_channels.send_dm(
                    &sender_public_key_hex,
                    crate::video_channels::DmVideoFrameMsg {
                        peer_pubkey: sender_public_key_hex.clone(),
                        stream_id_hex: hex::encode(stream_id),
                        frame_seq,
                        keyframe,
                        codec,
                        timestamp,
                        encoded_payload_b64: base64::engine::general_purpose::STANDARD
                            .encode(&data),
                    },
                );
            }
        }
    }
}
