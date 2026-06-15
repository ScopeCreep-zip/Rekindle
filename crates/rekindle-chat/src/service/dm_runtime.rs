//! ChatDmRuntime — implements DmDeps against ChatService's runtime types.
//!
//! Constructed once in `ChatService::new()`. This is the ONLY file that
//! imports both `dm/deps` types AND concrete runtime types (PlatformIO,
//! VaultStore, SessionCache, EventPipeline).
//!
//! The `mek_chains` field uses `parking_lot::RwLock` which produces
//! `Send` guards. Do not change to `std::sync::RwLock` — its guards
//! are not `Send` and would break async compatibility.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::RwLock;

use rekindle_storage::VaultStore;
use rekindle_types::dm_store::DmStore;
use rekindle_types::session_types::SessionMeta;
use rekindle_types::subscription_events::{
    ChannelMessageEvent, DmLifecycleEvent, SubscriptionEvent,
};
use rekindle_types::transport::RecordSchema;

use crate::dm::deps::{DmDeps, DmEvent, DmMekCache};
use crate::dm::error::DmError;
use crate::dm::DmMekChain;
use crate::events::pipeline::EventPipeline;
use crate::io::PlatformIO;
use crate::messaging::VaultSkippedCallback;

pub(super) struct ChatDmRuntime {
    pub(super) io: Arc<PlatformIO>,
    pub(super) vault: Arc<VaultStore>,
    pub(super) session_cache: Arc<crate::crypto::sessions::SessionCache>,
    pub(super) session_meta: Arc<RwLock<SessionMeta>>,
    pub(super) pipeline: Arc<EventPipeline>,
    pub(super) mek_chains: RwLock<HashMap<String, DmMekChain>>,
}

// ── DmMekCache ──────────────────────────────────────────────────────

impl DmMekCache for ChatDmRuntime {
    fn insert(&self, record_key: &str, chain: DmMekChain) {
        self.mek_chains
            .write()
            .insert(record_key.to_string(), chain);
    }

    fn current(&self, record_key: &str) -> Result<([u8; 32], u64), DmError> {
        let mut chains = self.mek_chains.write();
        let chain = chains
            .get_mut(record_key)
            .ok_or_else(|| DmError::MekChainUnavailable(record_key.into()))?;
        chain.current()
    }

    fn observed_and_lookup(
        &self,
        record_key: &str,
        observed_gen: u64,
    ) -> Result<[u8; 32], DmError> {
        let mut chains = self.mek_chains.write();
        let chain = chains
            .get_mut(record_key)
            .ok_or_else(|| DmError::MekChainUnavailable(record_key.into()))?;
        chain.observed_and_lookup(observed_gen)
    }

    fn advance(&self, record_key: &str) -> Result<u64, DmError> {
        let mut chains = self.mek_chains.write();
        let chain = chains
            .get_mut(record_key)
            .ok_or_else(|| DmError::MekChainUnavailable(record_key.into()))?;
        chain.advance()
    }
}

// ── DmDeps ──────────────────────────────────────────────────────────

#[async_trait]
impl DmDeps for ChatDmRuntime {
    // ── Identity ──────────────────────────────────────────

    fn identity_public_key_hex(&self) -> Result<String, DmError> {
        self.io
            .identity_public_key_hex()
            .map_err(|_| DmError::IdentityNotLoaded)
    }

    fn identity_public_key_bytes(&self) -> Result<[u8; 32], DmError> {
        self.io
            .identity_public_key_bytes()
            .map_err(|_| DmError::IdentityNotLoaded)
    }

    fn x25519_identity_seed(&self) -> Result<[u8; 32], DmError> {
        self.io
            .x25519_identity_seed()
            .map(|z| *z)
            .map_err(|e| DmError::EncryptFailed(format!("x25519 seed: {e}")))
    }

    // ── Persistence ───────────────────────────────────────

    fn store(&self) -> &dyn DmStore {
        &*self.vault
    }

    // ── Group MEK cache ───────────────────────────────────

    fn mek_cache(&self) -> &dyn DmMekCache {
        self
    }

    // ── 1:1 Triple Ratchet ────────────────────────────────

    async fn ratchet_encrypt(
        &self,
        peer_key: &str,
        plaintext: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), DmError> {
        let session_id = self
            .session_cache
            .ensure_loaded(peer_key)
            .await
            .map_err(|e| DmError::EncryptFailed(format!("session load: {e}")))?;
        tracing::debug!(
            peer = &peer_key[..12.min(peer_key.len())],
            session_id = hex::encode(&session_id[..8]),
            plaintext_len = plaintext.len(),
            "dm_runtime::ratchet_encrypt: session loaded"
        );

        let encrypted = self
            .session_cache
            .with_session(&session_id, |session| {
                rekindle_ratchet::ratchet::triple::encrypt(session, plaintext)
                    .map_err(crate::ChatError::from)
            })
            .await
            .map_err(|e| DmError::EncryptFailed(format!("triple encrypt: {e}")))?;

        self.session_cache
            .with_session(&session_id, |session| {
                self.session_cache.persist(&session_id, peer_key, session)
            })
            .await
            .map_err(|e| {
                tracing::error!(
                    peer = &peer_key[..12.min(peer_key.len())],
                    session_id = hex::encode(&session_id[..8]),
                    error = %e,
                    "dm_runtime::ratchet_encrypt: session persist FAILED — vault is stale"
                );
                DmError::Storage(format!("session persist after encrypt: {e}"))
            })?;

        Ok((encrypted.encrypted_header, encrypted.ciphertext))
    }

    async fn ratchet_decrypt(
        &self,
        peer_key: &str,
        encrypted_header: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, DmError> {
        let session_id = self
            .session_cache
            .ensure_loaded(peer_key)
            .await
            .map_err(|e| DmError::DecryptFailed(format!("session load: {e}")))?;
        tracing::debug!(
            peer = &peer_key[..12.min(peer_key.len())],
            session_id = hex::encode(&session_id[..8]),
            header_len = encrypted_header.len(),
            ct_len = ciphertext.len(),
            "dm_runtime::ratchet_decrypt: session loaded"
        );

        let plaintext = self
            .session_cache
            .with_session(&session_id, |session| {
                let skipped = VaultSkippedCallback {
                    vault: &self.vault,
                    session_id: &session_id,
                };
                rekindle_ratchet::ratchet::triple::decrypt(
                    session,
                    encrypted_header,
                    ciphertext,
                    &skipped,
                )
                .map_err(crate::ChatError::from)
            })
            .await
            .map_err(|e| DmError::DecryptFailed(format!("triple decrypt: {e}")))?;

        self.session_cache
            .with_session(&session_id, |session| {
                self.session_cache.persist(&session_id, peer_key, session)
            })
            .await
            .map_err(|e| {
                tracing::error!(
                    peer = &peer_key[..12.min(peer_key.len())],
                    session_id = hex::encode(&session_id[..8]),
                    error = %e,
                    "dm_runtime::ratchet_decrypt: session persist FAILED — vault is stale"
                );
                DmError::Storage(format!("session persist after decrypt: {e}"))
            })?;

        Ok(plaintext)
    }

    // ── DHT operations ────────────────────────────────────

    async fn dht_create_smpl_record(
        &self,
        member_pubkeys: Vec<[u8; 32]>,
    ) -> Result<String, DmError> {
        let schema = RecordSchema::MultiWriter {
            owner_subkeys: 0,
            member_subkeys: 1,
            member_keys: member_pubkeys,
        };
        let (record, _keypair) = self
            .io
            .create_record(schema)
            .await
            .map_err(|e| DmError::Transport(format!("create smpl record: {e}")))?;
        Ok(record.key().to_string())
    }

    async fn dht_open_record(&self, record_key: &str) -> Result<(), DmError> {
        self.io
            .open_record(record_key, None)
            .await
            .map(|_| ())
            .map_err(|e| DmError::Transport(format!("open record: {e}")))
    }

    async fn dht_write_subkey(
        &self,
        record_key: &str,
        subkey: u32,
        value: Vec<u8>,
        writer_keypair: ([u8; 32], [u8; 32]),
    ) -> Result<(), DmError> {
        // Veilid KeyPair wire format: [pub(32) || secret(32)]
        let mut kp_bytes = [0u8; 64];
        kp_bytes[..32].copy_from_slice(&writer_keypair.1); // public first
        kp_bytes[32..].copy_from_slice(&writer_keypair.0); // secret second
        self.io
            .open_and_write(
                record_key,
                subkey,
                &value,
                Some(&kp_bytes),
                crate::io::Confirm::Accepted,
            )
            .await
            .map(|_| ())
            .map_err(|e| DmError::Transport(format!("write subkey: {e}")))
    }

    async fn dht_read_subkey(
        &self,
        record_key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, DmError> {
        self.io
            .open_and_read(record_key, subkey, force_refresh)
            .await
            .map_err(|e| DmError::Transport(format!("read subkey: {e}")))
    }

    async fn dht_watch_subkeys(
        &self,
        record_key: &str,
        subkeys: Vec<u32>,
    ) -> Result<(), DmError> {
        let record = self.io
            .open_record(record_key, None)
            .await
            .map_err(|e| DmError::Transport(format!("open for watch: {e}")))?;
        self.io
            .watch_record(&record, &subkeys)
            .await
            .map(|_| ())
            .map_err(|e| DmError::Transport(format!("watch subkeys: {e}")))
    }

    // ── Peer key discovery ────────────────────────────────

    async fn read_peer_x25519_pub(
        &self,
        peer_profile_key: &str,
    ) -> Result<[u8; 32], DmError> {
        let profile_record = self.io
            .open_record(peer_profile_key, None)
            .await
            .map_err(|e| DmError::Transport(format!("open peer profile: {e}")))?;

        let data = self
            .io
            .read_record(
                &profile_record,
                rekindle_types::dht_types::PROFILE_SUBKEY_X25519_PUB,
                true,
            )
            .await
            .map_err(|e| DmError::Transport(format!("read x25519 pub: {e}")))?
            .ok_or_else(|| {
                DmError::Transport(format!(
                    "peer {} has no X25519 pub in profile",
                    &peer_profile_key[..20.min(peer_profile_key.len())]
                ))
            })?;

        let bytes: [u8; 32] = data.try_into().map_err(|_| {
            DmError::InvalidInput("X25519 pub must be 32 bytes".into())
        })?;
        Ok(bytes)
    }

    // ── Transport ─────────────────────────────────────────

    async fn send_app_call(
        &self,
        peer_pubkey_hex: &str,
        payload: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, DmError> {
        tokio::time::timeout(
            timeout,
            self.io.transport().call_peer(peer_pubkey_hex, payload),
        )
        .await
        .map_err(|_| {
            DmError::Transport(format!(
                "app_call to {}… timed out after {}s",
                &peer_pubkey_hex[..12.min(peer_pubkey_hex.len())],
                timeout.as_secs(),
            ))
        })?
        .map_err(|e| DmError::Transport(format!("app_call: {e}")))
    }

    async fn write_dm_invite_to_inbox(
        &self,
        peer_pubkey_hex: &str,
        invite_payload: &[u8],
    ) -> Result<(), DmError> {
        // Read peer's profile for inbox key + keypair
        let peer_profile = self.io
            .open_record(peer_pubkey_hex, None)
            .await
            .map_err(|e| DmError::Transport(format!("open peer profile: {e}")))?;

        let inbox_key = self
            .io
            .read_record(
                &peer_profile,
                rekindle_types::dht_types::PROFILE_SUBKEY_FRIEND_INBOX_KEY,
                true,
            )
            .await
            .map_err(|e| DmError::Transport(format!("read inbox key: {e}")))?
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();

        let inbox_kp_hex = self
            .io
            .read_record(
                &peer_profile,
                rekindle_types::dht_types::PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
                true,
            )
            .await
            .map_err(|e| DmError::Transport(format!("read inbox keypair: {e}")))?
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();

        if inbox_key.is_empty() || inbox_kp_hex.is_empty() {
            return Err(DmError::Transport(
                "dm_runtime: peer inbox key or keypair not available".into(),
            ));
        }

        let inbox_kp_bytes = hex::decode(&inbox_kp_hex)
            .map_err(|e| DmError::InvalidInput(format!("inbox keypair hex: {e}")))?;

        let our_pub = self
            .io
            .identity_public_key_hex()
            .map_err(|e| DmError::EncryptFailed(format!("identity pub hex: {e}")))?;

        let subkey =
            crate::friendship::request::blake3_hash_mod(&our_pub, peer_pubkey_hex, 32);

        let inbox_record = self.io
            .open_record(&inbox_key, Some(&inbox_kp_bytes))
            .await
            .map_err(|e| DmError::Transport(format!("open inbox record: {e}")))?;

        let existing = self
            .io
            .read_record(&inbox_record, subkey, true)
            .await
            .map_err(|e| DmError::Transport(format!("read inbox subkey: {e}")))?
            .unwrap_or_default();

        let mut entries: Vec<serde_json::Value> =
            if existing.is_empty() || existing == b"[]" {
                Vec::new()
            } else {
                serde_json::from_slice(&existing).unwrap_or_default()
            };

        let invite_value: serde_json::Value = serde_json::from_slice(invite_payload)
            .map_err(|e| DmError::InvalidInput(format!("invite json parse: {e}")))?;
        entries.push(invite_value);

        let bytes = serde_json::to_vec(&entries)
            .map_err(|e| DmError::InvalidInput(format!("entries json serialize: {e}")))?;

        self.io
            .write_record(
                &inbox_record,
                subkey,
                &bytes,
                Some(&inbox_kp_bytes),
                crate::io::Confirm::Accepted,
            )
            .await
            .map(|_| ())
            .map_err(|e| DmError::Transport(format!("write inbox: {e}")))
    }

    // ── Event emission ────────────────────────────────────

    fn emit_event(&self, event: DmEvent) {
        let sub_event = match event {
            DmEvent::MessageReceived {
                peer_key,
                sender_pseudonym,
                body,
                timestamp_ms,
                is_self,
                ..
            } => {
                let display_name = if is_self {
                    let meta = self.session_meta.read();
                    meta.identity.as_ref()
                        .map(|id| id.display_name.clone())
                        .unwrap_or(sender_pseudonym.clone())
                } else {
                    let meta = self.session_meta.read();
                    meta.friend_display_names.get(&sender_pseudonym)
                        .cloned()
                        .unwrap_or(sender_pseudonym.clone())
                };
                SubscriptionEvent::ChannelMessage(
                    ChannelMessageEvent::DirectMessageReceived {
                        peer_key,
                        timestamp: timestamp_ms,
                        sender_name: Some(display_name),
                        body: Some(body),
                        is_self,
                    },
                )
            }
            DmEvent::InviteReceived {
                record_key,
                sender_pseudonym,
                sender_public_key_hex,
                is_group,
            } => SubscriptionEvent::Dm(DmLifecycleEvent::InviteReceived {
                record_key,
                sender_pseudonym,
                sender_public_key_hex,
                is_group,
            }),
            DmEvent::InviteDeclined {
                record_key,
                reason,
            } => SubscriptionEvent::Dm(DmLifecycleEvent::InviteDeclined {
                record_key,
                reason,
            }),
            DmEvent::GroupMemberLeft {
                record_key,
                sender_public_key_hex,
            } => SubscriptionEvent::Dm(DmLifecycleEvent::MemberLeft {
                record_key,
                sender_public_key_hex,
            }),
        };
        self.pipeline.process(sub_event);
    }
}
