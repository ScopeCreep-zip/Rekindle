//! Phase 13 — DM domain adapter.
//!
//! Implements `rekindle_dm::DmDeps` + `rekindle_dm::DmMekCache` against
//! the live `AppState`, `tauri::AppHandle`, `DbPool`, and Veilid
//! `RoutingContext`. Every veilid-core / Tauri / SQLite touch the DM
//! domain logic needs is realized here so `rekindle-dm` stays a
//! veilid-free, Tauri-free, AppState-free domain crate.
//!
//! Construct one `Arc<DmAdapter>` per session and hand it to
//! `rekindle_dm::send_dm_message`, `handle_dm_subkey_change`, etc. The
//! adapter cheaply clones the underlying `Arc<AppState>` + DbPool +
//! AppHandle handles.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_dm::{
    DmDeps, DmError, DmEvent, DmMekCache, DmMekChain, DmStore, SqliteDmStore,
};
use rekindle_protocol::messaging::envelope::MessagePayload;
use rekindle_records::schema;
use veilid_core::{
    BarePublicKey, BareSecretKey, KeyPair, PublicKey, RecordKey, ValueSubkeyRangeSet,
    CRYPTO_KIND_VLD0,
};

use crate::services::message_service;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;

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
    pool: DbPool,
    store: Arc<dyn DmStore>,
    mek_cache: Arc<dyn DmMekCache>,
}

impl DmAdapter {
    #[must_use]
    pub fn new(state: Arc<AppState>, app_handle: tauri::AppHandle, pool: DbPool) -> Arc<Self> {
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

#[async_trait]
impl DmDeps for DmAdapter {
    fn owner_key(&self) -> Result<String, DmError> {
        let key = state_helpers::owner_key_or_default(&self.state);
        if key.is_empty() {
            Err(DmError::IdentityNotLoaded)
        } else {
            Ok(key)
        }
    }

    fn identity_secret(&self) -> Result<[u8; 32], DmError> {
        let guard = self.state.identity_secret.lock();
        guard.as_ref().copied().ok_or(DmError::IdentityNotLoaded)
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
    ) -> Result<String, DmError> {
        let rc = state_helpers::safe_routing_context(&self.state)
            .ok_or(DmError::RoutingContextUnavailable)?;
        let smpl_schema = schema::community_smpl_schema(&member_pubkeys)
            .map_err(|e| DmError::transport(format!("dm smpl schema: {e}")))?;
        let descriptor = rc
            .create_dht_record(CRYPTO_KIND_VLD0, smpl_schema, None)
            .await
            .map_err(|e| DmError::transport(format!("create dm dht record: {e}")))?;
        Ok(descriptor.key().to_string())
    }

    async fn dht_open_record(
        &self,
        record_key: &str,
        writer_keypair: Option<([u8; 32], [u8; 32])>,
    ) -> Result<(), DmError> {
        let rc = state_helpers::safe_routing_context(&self.state)
            .ok_or(DmError::RoutingContextUnavailable)?;
        let record_key_typed = record_key
            .parse::<RecordKey>()
            .map_err(|e| DmError::transport(format!("invalid record key: {e}")))?;
        let veilid_keypair = writer_keypair.map(|(secret, public)| {
            let veilid_pub =
                PublicKey::new(CRYPTO_KIND_VLD0, BarePublicKey::new(&public));
            let veilid_secret = BareSecretKey::new(&secret);
            KeyPair::new_from_parts(veilid_pub, veilid_secret)
        });
        let _ = rc
            .open_dht_record(record_key_typed, veilid_keypair)
            .await
            .map_err(|e| DmError::transport(format!("open dm record: {e}")))?;
        Ok(())
    }

    async fn dht_write_subkey(
        &self,
        record_key: &str,
        subkey: u32,
        value: Vec<u8>,
        _writer_keypair: ([u8; 32], [u8; 32]),
    ) -> Result<(), DmError> {
        // The writer keypair is already passed to dht_open_record above.
        // Veilid's set_dht_value uses the record's current writer
        // keypair (the one open_dht_record was called with), so we
        // don't need to pass it again here.
        let rc = state_helpers::safe_routing_context(&self.state)
            .ok_or(DmError::RoutingContextUnavailable)?;
        let record_key_typed = record_key
            .parse::<RecordKey>()
            .map_err(|e| DmError::transport(format!("invalid record key: {e}")))?;
        rc.set_dht_value(record_key_typed, subkey, value, None)
            .await
            .map_err(|e| DmError::transport(format!("write dm subkey: {e}")))?;
        Ok(())
    }

    async fn dht_watch_subkeys(
        &self,
        record_key: &str,
        subkeys: Vec<u32>,
    ) -> Result<(), DmError> {
        let rc = state_helpers::safe_routing_context(&self.state)
            .ok_or(DmError::RoutingContextUnavailable)?;
        let record_key_typed = record_key
            .parse::<RecordKey>()
            .map_err(|e| DmError::transport(format!("invalid record key: {e}")))?;
        // Build a subkey range set from the listed individual subkeys.
        let mut range = ValueSubkeyRangeSet::new();
        for sk in subkeys {
            range.insert(sk);
        }
        let _ = rc
            .watch_dht_values(record_key_typed, Some(range), None, None)
            .await
            .map_err(|e| DmError::transport(format!("watch dm subkeys: {e}")))?;
        Ok(())
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
        message_service::send_to_peer_encrypted(
            &self.state,
            &self.pool,
            peer_pubkey_hex,
            &payload,
        )
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
                crate::event_dispatch::emit_live(
                    &self.app_handle,
                    "chat-event",
                    &ChatEvent::MessageReceived {
                        from: sender_pseudonym,
                        body,
                        decryption_failed: false,
                        automod_blurred: false,
                        timestamp: timestamp_ms,
                        conversation_id: record_key,
                        server_message_id: None,
                        reply_to_id: None,
                        sender_display_name: None,
                    },
                );
            }
            DmEvent::InviteReceived {
                record_key,
                sender_pseudonym,
                sender_public_key_hex,
                is_group,
            } => {
                crate::event_dispatch::emit_live(
                    &self.app_handle,
                    "chat-event",
                    &ChatEvent::DirectMessageInvite {
                        from: sender_public_key_hex,
                        record_key,
                        initiator_pseudonym: sender_pseudonym,
                        is_group,
                    },
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
                timestamp,
                data,
            } => {
                use base64::Engine as _;
                let payload = serde_json::json!({
                    "kind": "dmVideoFrame",
                    "peerPubkey": sender_public_key_hex,
                    "streamIdHex": hex::encode(stream_id),
                    "frameSeq": frame_seq,
                    "keyframe": keyframe,
                    "timestamp": timestamp,
                    "encodedPayloadB64": base64::engine::general_purpose::STANDARD.encode(&data),
                });
                crate::event_dispatch::dispatch(&self.app_handle, "dm-video-frame", payload);
            }
        }
    }
}

