//! Phase 15 — `MockDeps` fixture implementing `FilesDeps` for crate
//! unit tests. Held in-tree (rather than dev-dependencies) so any
//! crate test can exercise the upload/download/expression_fetch
//! flows against deterministic state.
//!
//! State is held under `parking_lot::Mutex` so the trait methods
//! (all `&self`) can mutate. The cache is a real `ChunkCache` rooted
//! at a `TempDir` so chunk insert/get/bitmap behave exactly like
//! production.

#![cfg(test)]

use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::channel_record::{
    ChannelAttachmentCached, ChannelMessage, ChannelRecordEntry,
};
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use tempfile::TempDir;
use uuid::Uuid;

use crate::cache::{CacheConfig, ChunkCache};
use crate::deps::{FilesDeps, FilesEvent};
use crate::error::FilesError;
use crate::pinned::PinnedSet;
use rekindle_types::attachment::AttachmentOffer;

/// Records of mutations + emissions for test assertions.
#[derive(Default)]
pub struct MockCalls {
    pub channel_messages_written: Vec<ChannelMessage>,
    pub attachment_cacheds_written: Vec<ChannelAttachmentCached>,
    pub channel_messages_persisted: Vec<String>, // message_id
    pub slowmode_persists: Vec<(String, String, i64)>, // community, channel, now_ms
    pub local_path_persists: Vec<(String, String, String)>, // owner, channel, attachment_id_hex
    pub attachment_pinneds: Vec<([u8; 16], bool, u64)>, // attachment_id, pinned, lamport
    pub events: Vec<FilesEvent>,
    pub app_calls: Vec<Vec<u8>>, // routed payloads
}

/// Programmable app_call reply queue — pop FIFO.
#[derive(Default)]
pub struct MockReplies {
    pub queue: std::collections::VecDeque<Result<Vec<u8>, FilesError>>,
}

#[allow(dead_code, reason = "kept for symmetry + future test cases needing per-mock identity assertions")]
pub struct MockDeps {
    pub community_id: String,
    pub channel_id: String,
    pub owner_key: String,
    pub my_pseudonym: String,
    pub channel_log_key: String,
    pub slot_keypair: String,
    pub my_subkey_index: u32,
    pub mek_generation: u64,
    pub forum_channel: bool,
    pub permission_pass: bool,
    pub slowmode_pass: bool,

    /// Current channel MEK (also serves as community MEK for tests).
    pub channel_mek: Option<MediaEncryptionKey>,
    /// (community_id, channel_id, generation) -> MEK for the historical cascade.
    pub historical_meks: HashMap<u64, MediaEncryptionKey>,

    /// Online member route blobs (pseudonym -> blob).
    pub peers: HashMap<String, Vec<u8>>,

    /// Pre-loaded entries the scan_channel_subkeys call returns.
    pub channel_entries: Mutex<Vec<ChannelRecordEntry>>,

    /// Governance: pinned attachments + expressions.
    pub pinned_attachments: Vec<Uuid>,
    pub expressions: Vec<([u8; 16], AttachmentOffer)>,

    /// Real ChunkCache rooted at temp dir.
    pub cache: Mutex<ChunkCache>,
    /// PinnedSet for chunk inserts.
    pub pinned: PinnedSet,
    /// Holds the TempDir alive (drop with the mock).
    pub _tempdir: TempDir,

    /// Recorded mutations.
    pub calls: Mutex<MockCalls>,
    /// Programmable app_call_peer replies.
    pub replies: Mutex<MockReplies>,
}

impl MockDeps {
    /// Construct with reasonable defaults; tests override fields directly.
    pub fn new(community_id: &str, channel_id: &str) -> Self {
        let temp = TempDir::new().unwrap();
        let cache = ChunkCache::open(CacheConfig {
            root_dir: temp.path().to_path_buf(),
            byte_budget: 4 * 1024 * 1024,
        })
        .unwrap();
        Self {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            owner_key: "owner-pub-key".to_string(),
            my_pseudonym: "me-pseudonym".to_string(),
            channel_log_key: "channel-log-key-hex".to_string(),
            slot_keypair: "stub-slot-keypair".to_string(),
            my_subkey_index: 5,
            mek_generation: 1,
            forum_channel: false,
            permission_pass: true,
            slowmode_pass: true,
            channel_mek: None,
            historical_meks: HashMap::new(),
            peers: HashMap::new(),
            channel_entries: Mutex::new(Vec::new()),
            pinned_attachments: Vec::new(),
            expressions: Vec::new(),
            cache: Mutex::new(cache),
            pinned: PinnedSet::new(),
            _tempdir: temp,
            calls: Mutex::new(MockCalls::default()),
            replies: Mutex::new(MockReplies::default()),
        }
    }

    /// Install a channel MEK at the current generation + cascade lookup.
    pub fn with_mek(mut self, generation: u64, key_bytes: [u8; 32]) -> Self {
        let mek = MediaEncryptionKey::from_bytes(key_bytes, generation);
        self.mek_generation = generation;
        self.channel_mek = Some(mek.clone());
        self.historical_meks.insert(generation, mek);
        self
    }

    pub fn with_peer(mut self, pseudonym: &str, route_blob: Vec<u8>) -> Self {
        self.peers.insert(pseudonym.to_string(), route_blob);
        self
    }

    pub fn with_entries(self, entries: Vec<ChannelRecordEntry>) -> Self {
        *self.channel_entries.lock() = entries;
        self
    }
}

#[async_trait]
impl FilesDeps for MockDeps {
    fn owner_key(&self) -> Result<String, FilesError> {
        Ok(self.owner_key.clone())
    }

    fn my_pseudonym(&self, _community_id: &str) -> Result<String, FilesError> {
        Ok(self.my_pseudonym.clone())
    }

    fn channel_log_key(&self, _c: &str, _ch: &str) -> Result<String, FilesError> {
        Ok(self.channel_log_key.clone())
    }

    fn slot_keypair(&self, _c: &str) -> Result<String, FilesError> {
        Ok(self.slot_keypair.clone())
    }

    fn my_subkey_index(&self, _c: &str) -> Result<u32, FilesError> {
        Ok(self.my_subkey_index)
    }

    fn channel_is_forum(&self, _c: &str, _ch: &str) -> bool {
        self.forum_channel
    }

    fn mek_generation(&self, _c: &str) -> Result<u64, FilesError> {
        Ok(self.mek_generation)
    }

    fn channel_mek(&self, c: &str, _ch: &str) -> Result<MediaEncryptionKey, FilesError> {
        self.channel_mek.clone().ok_or(FilesError::MekUnavailable {
            community: c.to_string(),
            generation: self.mek_generation,
        })
    }

    fn historical_channel_mek(
        &self,
        _c: &str,
        _ch: &str,
        generation: u64,
    ) -> Option<MediaEncryptionKey> {
        self.historical_meks.get(&generation).cloned()
    }

    fn community_mek(&self, _c: &str) -> Option<MediaEncryptionKey> {
        self.channel_mek.clone()
    }

    fn require_permission(&self, _c: &str, _p: Permissions) -> Result<(), FilesError> {
        if self.permission_pass {
            Ok(())
        } else {
            Err(FilesError::PermissionDenied("mock denied".into()))
        }
    }

    fn enforce_slowmode(&self, _c: &str, _ch: &str, _now_ms: i64) -> Result<(), FilesError> {
        if self.slowmode_pass {
            Ok(())
        } else {
            Err(FilesError::Slowmode("mock slowmode".into()))
        }
    }

    fn resolve_outbound_mentions(
        &self,
        _c: &str,
        _sender: &str,
        _body: &str,
    ) -> (Vec<String>, Vec<String>, u32) {
        (Vec::new(), Vec::new(), 0)
    }

    fn increment_lamport(&self, _c: &str) -> u64 {
        1
    }

    fn next_channel_sequence(&self, _c: &str, _ch: &str) -> u64 {
        1
    }

    fn ensure_cache_open(&self, _c: &str) -> Result<(), FilesError> {
        Ok(())
    }

    fn pinned_attachments(&self, _c: &str) -> PinnedSet {
        self.pinned.clone()
    }

    fn set_community_pinned_set(&self, _c: &str, _ids: Vec<uuid::Uuid>) {
        // MockDeps doesn't track per-community pinned overrides — the
        // single `pinned` field above is shared. Tests that exercise
        // sync_pinned_from_governance need a more elaborate mock.
    }

    fn with_cache_mut(
        &self,
        _c: &str,
        f: &mut dyn FnMut(&mut ChunkCache, &PinnedSet) -> Result<(), FilesError>,
    ) -> Result<(), FilesError> {
        let mut cache = self.cache.lock();
        f(&mut cache, &self.pinned)
    }

    fn with_cache(
        &self,
        _c: &str,
        f: &mut dyn FnMut(&ChunkCache) -> Result<(), FilesError>,
    ) -> Result<(), FilesError> {
        let cache = self.cache.lock();
        f(&cache)
    }

    fn peer_route_blob(&self, _c: &str, target: &str) -> Option<Vec<u8>> {
        self.peers.get(target).cloned()
    }

    fn online_member_pseudonyms(&self, _c: &str) -> Vec<String> {
        self.peers.keys().cloned().collect()
    }

    fn governance_pinned_attachments(&self, _c: &str) -> Vec<Uuid> {
        self.pinned_attachments.clone()
    }

    fn governance_expressions_with_attachments(
        &self,
        _c: &str,
    ) -> Vec<([u8; 16], AttachmentOffer)> {
        self.expressions.clone()
    }

    async fn write_channel_message_to_smpl(
        &self,
        _c: &str,
        _key: &str,
        _idx: u32,
        _keypair: &str,
        message: &ChannelMessage,
    ) -> Result<(), FilesError> {
        self.calls
            .lock()
            .channel_messages_written
            .push(message.clone());
        Ok(())
    }

    async fn write_attachment_cached_to_smpl(
        &self,
        _c: &str,
        _key: &str,
        _idx: u32,
        _keypair: &str,
        cached: &ChannelAttachmentCached,
    ) -> Result<(), FilesError> {
        self.calls
            .lock()
            .attachment_cacheds_written
            .push(cached.clone());
        Ok(())
    }

    async fn scan_channel_subkeys(
        &self,
        _key: &str,
    ) -> Result<Vec<ChannelRecordEntry>, FilesError> {
        Ok(self.channel_entries.lock().clone())
    }

    async fn app_call_peer(
        &self,
        _route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, FilesError> {
        self.calls.lock().app_calls.push(payload);
        let mut replies = self.replies.lock();
        replies
            .queue
            .pop_front()
            .unwrap_or_else(|| Err(FilesError::Transport("no mock reply queued".into())))
    }

    #[allow(clippy::too_many_arguments, reason = "matches trait surface")]
    async fn insert_channel_message_full(
        &self,
        _owner: &str,
        _channel: &str,
        _sender: &str,
        message_id: &str,
        _ts: i64,
        _gen: u64,
        _lamport: u64,
        _aj: &str,
        _flags: u32,
        _body: &str,
    ) -> Result<(), FilesError> {
        self.calls
            .lock()
            .channel_messages_persisted
            .push(message_id.to_string());
        Ok(())
    }

    fn persist_slowmode_state(&self, community_id: &str, channel_id: &str, now_ms: i64) {
        self.calls.lock().slowmode_persists.push((
            community_id.to_string(),
            channel_id.to_string(),
            now_ms,
        ));
    }

    async fn persist_local_path(
        &self,
        owner_key: &str,
        channel_id: &str,
        attachment_id_hex: &str,
        _save_path: &Path,
    ) -> Result<(), FilesError> {
        self.calls.lock().local_path_persists.push((
            owner_key.to_string(),
            channel_id.to_string(),
            attachment_id_hex.to_string(),
        ));
        Ok(())
    }

    fn send_to_mesh(
        &self,
        _c: &str,
        _envelope: &CommunityEnvelope,
    ) -> Result<(), FilesError> {
        Ok(())
    }

    async fn write_attachment_pinned(
        &self,
        _c: &str,
        attachment_id: [u8; 16],
        pinned: bool,
        lamport: u64,
    ) -> Result<(), FilesError> {
        self.calls
            .lock()
            .attachment_pinneds
            .push((attachment_id, pinned, lamport));
        Ok(())
    }

    fn emit_event(&self, event: FilesEvent) {
        self.calls.lock().events.push(event);
    }
}
