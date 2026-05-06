//! Lost Cargo (file sharing) service module.
//!
//! Architecture §28.9 — chunked attachment delivery over Veilid:
//!  - chunks live in a local per-community filesystem cache (`rekindle-files`)
//!  - the announcement (`AttachmentOffer`) travels embedded in a
//!    `ChannelEntry::Message` (architecture line 3233)
//!  - peers write `AttachmentCached` entries to their SMPL subkeys to
//!    advertise possession; downloaders scan those entries to find sources
//!  - chunks themselves move via `app_call` (`ControlPayload::AttachmentChunk`)
//!
//! This module is the wiring layer between `rekindle-files` (pure logic),
//! `rekindle-protocol` (wire types + DHT helpers), and the Tauri command
//! handlers in `commands/community/files.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_files::{
    validate_offer, verify_chunk, verify_merkle_root, AttachmentBitmap, AttachmentOffer,
    CacheConfig, ChunkCache, Chunker, CHUNK_SIZE_BYTES, MAX_FILE_SIZE_BYTES,
};
use rekindle_protocol::dht::community::channel_record::{
    ChannelAttachmentCached, ChannelMessage,
};
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::DHTManager;
use serde::Deserialize;
use tauri::AppHandle;
use tauri::Emitter;
use uuid::Uuid;

use crate::state::{AppState, SharedState};
use crate::state_helpers;

/// 1 GB per spec §28.9 line 3283.
const DEFAULT_BYTE_BUDGET: u64 = 1024 * 1024 * 1024;

// ─── Phase 3: cache lifecycle ──────────────────────────────────────────

/// Resolve the per-community cache directory under the global file_cache root.
fn community_cache_dir(state: &SharedState, community_id: &str) -> Option<PathBuf> {
    let root = state.file_cache_root.read().clone()?;
    Some(root.join(community_id))
}

/// Ensure the chunk cache for a given community is open. Idempotent.
pub fn ensure_cache_open(state: &SharedState, community_id: &str) -> Result<(), String> {
    if state.file_caches.read().contains_key(community_id) {
        return Ok(());
    }
    let dir = community_cache_dir(state, community_id)
        .ok_or_else(|| "file cache root not initialized".to_string())?;
    let cache = ChunkCache::open(CacheConfig {
        root_dir: dir,
        byte_budget: DEFAULT_BYTE_BUDGET,
    })
    .map_err(|e| format!("failed to open file cache: {e}"))?;
    state
        .file_caches
        .write()
        .entry(community_id.to_string())
        .or_insert(cache);
    state
        .pinned_attachments
        .write()
        .entry(community_id.to_string())
        .or_default();
    Ok(())
}

/// Sync the in-memory pinned set for a community from the merged governance
/// state's `pinned_attachments`. Run after every governance merge.
pub fn sync_pinned_from_governance(state: &SharedState, community_id: &str) {
    let pinned_ids: Vec<Uuid> = {
        let communities = state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return;
        };
        let Some(gov) = community.governance_state.as_ref() else {
            return;
        };
        gov.pinned_attachments
            .iter()
            .map(|bytes| Uuid::from_bytes(*bytes))
            .collect()
    };
    let mut all = state.pinned_attachments.write();
    let entry = all.entry(community_id.to_string()).or_default();
    entry.replace(pinned_ids);
}

// ─── Phase 4: upload ───────────────────────────────────────────────────

/// JSON payload stored in `messages.attachment_json` so the local UI can
/// render the file metadata without re-fetching the offer from the DHT.
#[derive(Debug, serde::Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentRecordJson {
    pub attachment_id: String,
    pub filename: String,
    pub mime_type: String,
    pub total_size: u64,
    pub chunk_count: u32,
    /// Set after a download completes locally; absent until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
}

/// Bundle of state computed once per upload — pseudonym, slot keypair,
/// channel record key, current channel MEK.
struct UploadContext {
    community_id: String,
    channel_key: String,
    slot_keypair: String,
    slot_index: u32,
    sender_pseudonym: String,
    mek_generation: u64,
    channel_mek: MediaEncryptionKey,
}

fn build_upload_context(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
) -> Result<UploadContext, String> {
    crate::commands::community::require_permission(
        state,
        community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::SEND_MESSAGES,
    )?;
    let communities = state.communities.read();
    let community = communities
        .get(community_id)
        .ok_or_else(|| "community not found".to_string())?;
    if community.channels.iter().any(|ch| {
        ch.id == channel_id && matches!(ch.channel_type, crate::state::ChannelType::Forum)
    }) {
        return Err("forum channels accept posts only through thread creation".to_string());
    }
    let channel_key = community
        .channel_log_keys
        .get(channel_id)
        .cloned()
        .ok_or_else(|| "channel record key missing".to_string())?;
    let slot_keypair = community
        .slot_keypair
        .clone()
        .ok_or_else(|| "slot keypair missing for community".to_string())?;
    let slot_index = community
        .my_subkey_index
        .ok_or_else(|| "subkey index missing for community".to_string())?;
    let sender_pseudonym = community
        .my_pseudonym_key
        .clone()
        .ok_or_else(|| "pseudonym missing for community".to_string())?;
    let mek_generation = community.mek_generation;
    drop(communities);

    let channel_mek = {
        let cache = state.channel_mek_cache.lock();
        if let Some(mek) =
            cache.get(&(community_id.to_string(), channel_id.to_string()))
        {
            mek.clone()
        } else {
            let community_mek = state
                .mek_cache
                .lock()
                .get(community_id)
                .cloned()
                .ok_or_else(|| {
                    "MEK not available — rejoin the community or wait for MEK delivery"
                        .to_string()
                })?;
            community_mek
        }
    };

    Ok(UploadContext {
        community_id: community_id.to_string(),
        channel_key,
        slot_keypair,
        slot_index,
        sender_pseudonym,
        mek_generation,
        channel_mek,
    })
}

/// Read a file from disk, chunk + encrypt under a fresh per-file FEK, store
/// chunks in the local cache, write the `AttachmentOffer` to the channel
/// SMPL record (embedded in a `ChannelEntry::Message`), gossip a
/// `MessageNotification`, and announce full possession via a
/// `ChannelAttachmentCached` write to our subkey.
///
/// Returns the new `attachment_id` (16-byte UUID, hex-encoded). Caller is
/// expected to have called `ensure_cache_open` for `community_id` already
/// (the join/login path does this automatically).
pub async fn upload_file(
    state: &SharedState,
    pool: &crate::db::DbPool,
    community_id: &str,
    channel_id: &str,
    file_path: &Path,
) -> Result<String, String> {
    let bytes = std::fs::read(file_path)
        .map_err(|e| format!("failed to read file '{}': {e}", file_path.display()))?;
    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let mime_type = guess_mime_type(file_path);
    upload_bytes_as_attachment(
        state,
        pool,
        community_id,
        channel_id,
        bytes,
        filename,
        mime_type,
        b"",
        0,
        Some(file_path.display().to_string()),
    )
    .await
}

/// Core upload helper: chunk + FEK-encrypt + cache `bytes`, build the
/// `AttachmentOffer`, write the carrying `ChannelMessage` (with the given
/// `body_plaintext` MEK-encrypted into `ciphertext` and the given `flags`),
/// announce possession via `AttachmentCached`, gossip notify. Returns the
/// new attachment_id (hex).
///
/// Used by both regular file uploads (`upload_file`) and voice messages
/// (`send_voice_message_bytes`). The two callers differ only in `flags`
/// (VOICE_MESSAGE) and `body_plaintext` (voice metadata JSON).
#[allow(clippy::too_many_arguments)]
pub async fn upload_bytes_as_attachment(
    state: &SharedState,
    pool: &crate::db::DbPool,
    community_id: &str,
    channel_id: &str,
    bytes: Vec<u8>,
    filename: String,
    mime_type: String,
    body_plaintext: &[u8],
    flags: u32,
    local_path: Option<String>,
) -> Result<String, String> {
    let total_size = bytes.len() as u64;
    if total_size > MAX_FILE_SIZE_BYTES {
        return Err(format!(
            "file too large: {total_size} bytes (max {MAX_FILE_SIZE_BYTES})"
        ));
    }

    let ctx = build_upload_context(state, community_id, channel_id)?;
    // Architecture §28.7 — file uploads (and voice messages, which call
    // through here) write a ChannelMessage carrying the attachment, so
    // the slowmode gate applies just like a plain text send.
    crate::services::community::channel_messages::enforce_slowmode(
        state,
        community_id,
        channel_id,
        crate::db::timestamp_now(),
    )?;
    ensure_cache_open(state, community_id)?;

    // Per-file FEK (plan §1.J1): fresh AES-256 key, never reused.
    let fek = MediaEncryptionKey::generate(0);

    // Chunk the plaintext + compute chunk_hashes + Merkle root.
    let chunked = Chunker::chunk(&bytes).map_err(|e| format!("chunker failed: {e}"))?;
    let attachment_id = chunked.attachment_id;
    let attachment_uuid = Uuid::from_bytes(attachment_id);
    let chunk_count = u32::try_from(chunked.chunks.len())
        .map_err(|_| "chunk count exceeds u32::MAX".to_string())?;

    // Encrypt + cache each chunk.
    {
        let mut caches = state.file_caches.write();
        let cache = caches
            .get_mut(community_id)
            .ok_or_else(|| "file cache not open for community".to_string())?;
        let pinned_lock = state.pinned_attachments.read();
        let pinned = pinned_lock
            .get(community_id)
            .cloned()
            .unwrap_or_default();
        for (idx, chunk) in chunked.chunks.iter().enumerate() {
            let ciphertext = fek
                .encrypt(chunk)
                .map_err(|e| format!("FEK chunk encrypt failed at {idx}: {e}"))?;
            let chunk_idx = u32::try_from(idx)
                .map_err(|_| "chunk index exceeds u32::MAX".to_string())?;
            cache
                .insert(attachment_uuid, chunk_idx, &ciphertext, &pinned)
                .map_err(|e| format!("cache insert failed at {chunk_idx}: {e}"))?;
        }
    }

    // Wrap FEK under the channel MEK at upload time (plan §1.J1).
    let wrapped_fek = ctx
        .channel_mek
        .encrypt(fek.as_bytes())
        .map_err(|e| format!("wrap FEK failed: {e}"))?;

    let offer = AttachmentOffer {
        attachment_id,
        filename: filename.clone(),
        mime_type: mime_type.clone(),
        total_size,
        chunk_count,
        chunk_size: u32::try_from(CHUNK_SIZE_BYTES).unwrap_or(u32::MAX),
        merkle_root: chunked.merkle_root,
        chunk_hashes: chunked.chunk_hashes,
        wrapped_fek,
        fek_mek_generation: ctx.mek_generation,
    };
    validate_offer(&offer).map_err(|e| format!("offer self-check failed: {e}"))?;

    // Carrying ChannelMessage. The body holds whatever the caller asked
    // (empty for plain uploads; JSON metadata for voice messages).
    let timestamp_ms = crate::db::timestamp_now();
    let message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let lamport_ts = state_helpers::increment_lamport(state, community_id);
    let sequence = next_channel_sequence(state, community_id, channel_id);
    let body_ciphertext = ctx
        .channel_mek
        .encrypt(body_plaintext)
        .map_err(|e| format!("MEK encrypt body: {e}"))?;

    // Architecture §28.5 — caption mentions go into the cleartext
    // envelope. Voice messages and file uploads can still ping people
    // via the body text. The MENTION_EVERYONE/MENTION_HERE flag bits
    // are OR'd into whatever flags the caller already passed (e.g.
    // VOICE_MESSAGE).
    let body_text = String::from_utf8_lossy(body_plaintext);
    let (mentioned_pseudonyms, mentioned_roles, mention_flags) =
        crate::services::community::channel_messages::resolve_outbound_mentions(
            state,
            community_id,
            &ctx.sender_pseudonym,
            &body_text,
        );

    let channel_msg = ChannelMessage {
        sequence,
        sender_pseudonym: ctx.sender_pseudonym.clone(),
        ciphertext: body_ciphertext,
        mek_generation: ctx.mek_generation,
        timestamp: u64::try_from(timestamp_ms).unwrap_or_default(),
        reply_to: None,
        lamport_ts,
        message_id: Some(message_id.clone()),
        attachment: Some(offer.clone()),
        flags: flags | mention_flags,
        mentioned_pseudonyms,
        mentioned_roles,
    };

    // Persist a SQLite row.
    let owner_key = state_helpers::current_owner_key(state)?;
    let attachment_json = serde_json::to_string(&AttachmentRecordJson {
        attachment_id: hex::encode(attachment_id),
        filename: filename.clone(),
        mime_type: mime_type.clone(),
        total_size,
        chunk_count,
        local_path,
    })
    .map_err(|e| format!("failed to serialize attachment_json: {e}"))?;
    let body_for_db = if flags & rekindle_types::channel::flags::VOICE_MESSAGE != 0 {
        // Voice-message body is JSON metadata: persist it as-is so the local
        // SQLite read path can decode without a separate fetch.
        String::from_utf8_lossy(body_plaintext).to_string()
    } else {
        String::new()
    };
    insert_message_full_attachment(
        pool,
        &owner_key,
        channel_id,
        &ctx.sender_pseudonym,
        &message_id,
        timestamp_ms,
        ctx.mek_generation,
        lamport_ts,
        &attachment_json,
        flags,
        &body_for_db,
    )
    .await?;

    // SMPL write — embed offer in a Message entry.
    let writer = ctx
        .slot_keypair
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| format!("invalid slot keypair: {e}"))?;
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let mgr = DHTManager::new(rc);
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(state, &ctx.community_id)?;
    rekindle_protocol::dht::community::channel_record::write_member_message(
        &mgr,
        &ctx.channel_key,
        ctx.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        &channel_msg,
    )
    .await
    .map_err(|e| format!("SMPL channel write failed: {e}"))?;

    // Architecture §28.7 — record this send for the slowmode window so
    // the next call to enforce_slowmode (from any send path) sees the
    // updated timestamp. Persisted via the same SQLite table the text
    // send path uses.
    {
        let now_ms = crate::db::timestamp_now();
        {
            let mut communities = state.communities.write();
            if let Some(cs) = communities.get_mut(&ctx.community_id) {
                cs.channel_last_send_at
                    .insert(channel_id.to_string(), now_ms);
            }
        }
        let owner_for_db = state_helpers::owner_key_or_default(state);
        if !owner_for_db.is_empty() {
            let community_for_db = ctx.community_id.clone();
            let channel_for_db = channel_id.to_string();
            crate::db_helpers::db_fire(pool, "persist channel_slowmode_state (attachment)", move |conn| {
                conn.execute(
                    "INSERT INTO channel_slowmode_state \
                     (owner_key, community_id, channel_id, last_send_ms) \
                     VALUES (?1, ?2, ?3, ?4) \
                     ON CONFLICT(owner_key, community_id, channel_id) DO UPDATE SET \
                       last_send_ms = excluded.last_send_ms",
                    rusqlite::params![owner_for_db, community_for_db, channel_for_db, now_ms],
                )?;
                Ok(())
            });
        }
    }

    // Announce full possession via AttachmentCached entry on our subkey.
    write_self_attachment_cached(
        state,
        &ctx,
        attachment_id,
        chunk_count,
        AttachmentBitmap::full(chunk_count),
    )
    .await?;

    let notification = CommunityEnvelope::MessageNotification {
        channel_id: channel_id.to_string(),
        message_id: message_id.clone(),
        author_pseudonym: ctx.sender_pseudonym.clone(),
        subkey_index: crate::services::community::channel_message_subkey(ctx.slot_index),
        lamport_ts,
        sequence,
        content_hash: blake3::hash(&channel_msg.ciphertext).to_hex().to_string(),
        timestamp: channel_msg.timestamp,
    };
    crate::services::community::send_to_mesh(state, community_id, &notification)?;

    Ok(hex::encode(attachment_id))
}

// ─── Phase 4 (voice messages): send a recorded voice clip ──────────────

/// Architecture §16.4 — a voice message is a `ChannelEntry::Message` with
/// `flags |= VOICE_MESSAGE` carrying a single `audio/ogg` Lost Cargo
/// attachment, plus waveform + duration metadata. The metadata travels
/// inside the (MEK-encrypted) body as JSON so it propagates with the
/// message without changing the AttachmentOffer wire format. Receivers
/// branch on the `VOICE_MESSAGE` flag to decode the body and render a
/// player.
///
/// `opus_bytes` is the recorded Opus-in-OGG container produced by the
/// frontend's MediaRecorder. `duration_ms` and `waveform` (≤256 peak
/// bytes) are computed alongside.
pub async fn send_voice_message_bytes(
    state: &SharedState,
    pool: &crate::db::DbPool,
    community_id: &str,
    channel_id: &str,
    opus_bytes: Vec<u8>,
    duration_ms: u32,
    waveform: Vec<u8>,
) -> Result<String, String> {
    crate::commands::community::require_permission(
        state,
        community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::SEND_VOICE_MESSAGES,
    )?;
    if duration_ms == 0 || duration_ms > 5 * 60 * 1000 {
        return Err(format!(
            "voice message duration {duration_ms}ms outside 1ms..=5min"
        ));
    }
    if waveform.len() > 256 {
        return Err(format!(
            "waveform has {} peaks — max 256",
            waveform.len()
        ));
    }
    if opus_bytes.is_empty() {
        return Err("voice message has empty audio bytes".to_string());
    }

    // Body JSON: peers decode this after MEK-decrypting the message body.
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct VoiceMessageBody {
        duration_ms: u32,
        waveform_b64: String,
    }
    use base64::Engine as _;
    let body = serde_json::to_vec(&VoiceMessageBody {
        duration_ms,
        waveform_b64: base64::engine::general_purpose::STANDARD.encode(&waveform),
    })
    .map_err(|e| format!("serialize voice message body: {e}"))?;

    upload_bytes_as_attachment(
        state,
        pool,
        community_id,
        channel_id,
        opus_bytes,
        format!("voice-{}.ogg", uuid::Uuid::new_v4().simple()),
        "audio/ogg".to_string(),
        &body,
        rekindle_types::channel::flags::VOICE_MESSAGE,
        None,
    )
    .await
}

async fn write_self_attachment_cached(
    state: &SharedState,
    ctx: &UploadContext,
    attachment_id: [u8; 16],
    chunk_count: u32,
    bitmap: AttachmentBitmap,
) -> Result<(), String> {
    let lamport_ts = state_helpers::increment_lamport(state, &ctx.community_id);
    let cached = ChannelAttachmentCached {
        attachment_id,
        chunk_bitmap: bitmap.as_bytes().to_vec(),
        chunk_count,
        author_pseudonym: ctx.sender_pseudonym.clone(),
        lamport_ts,
    };
    let writer = ctx
        .slot_keypair
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| format!("invalid slot keypair: {e}"))?;
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let mgr = DHTManager::new(rc);
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(state, &ctx.community_id)?;
    rekindle_protocol::dht::community::channel_record::write_member_attachment_cached(
        &mgr,
        &ctx.channel_key,
        ctx.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        &cached,
    )
    .await
    .map_err(|e| format!("AttachmentCached SMPL write failed: {e}"))?;
    Ok(())
}

fn next_channel_sequence(state: &SharedState, community_id: &str, channel_id: &str) -> u64 {
    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        let s = cs
            .channel_sequences
            .entry(channel_id.to_string())
            .or_insert(0);
        *s += 1;
        *s
    } else {
        1
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_message_full_attachment(
    pool: &crate::db::DbPool,
    owner_key: &str,
    channel_id: &str,
    sender_key: &str,
    message_id: &str,
    timestamp_ms: i64,
    mek_generation: u64,
    lamport_ts: u64,
    attachment_json: &str,
    flags: u32,
    body: &str,
) -> Result<(), String> {
    let mek_generation = i64::try_from(mek_generation).unwrap_or(i64::MAX);
    let owner = owner_key.to_string();
    let chan = channel_id.to_string();
    let sender = sender_key.to_string();
    let mid = message_id.to_string();
    let attachment_json = attachment_json.to_string();
    let body = body.to_string();
    crate::db_helpers::db_call(pool, move |conn| {
        crate::message_repo::insert_channel_message_full(
            conn,
            &owner,
            &chan,
            &sender,
            &body,
            timestamp_ms,
            true,
            Some(mek_generation),
            &mid,
            lamport_ts,
            false,
            None,
            flags,
            Some(&attachment_json),
        )
    })
    .await
    .map_err(|e| format!("db insert attachment row: {e}"))
}

fn guess_mime_type(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase) {
        Some(ext) => match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "mp4" => "video/mp4",
            "webm" => "video/webm",
            "mp3" => "audio/mpeg",
            "ogg" | "opus" => "audio/ogg",
            "pdf" => "application/pdf",
            "txt" => "text/plain",
            "json" => "application/json",
            "zip" => "application/zip",
            _ => "application/octet-stream",
        }
        .to_string(),
        None => "application/octet-stream".to_string(),
    }
}

// ─── Phase 4: serve chunks (responder side of app_call) ────────────────

/// Handle an incoming `RequestAttachment` control payload — a peer wants
/// chunks of an attachment we may have cached. We reply with a
/// `MultiAttachmentChunk` envelope containing each chunk we hold from the
/// requested set. Returns the serialized reply bytes (an
/// `app_call_reply` payload) — `None` if we have nothing to offer.
pub fn serve_attachment_request(
    state: &Arc<AppState>,
    community_id: &str,
    attachment_id: [u8; 16],
    requested_chunks: &[u32],
) -> Option<Vec<u8>> {
    let attachment_uuid = Uuid::from_bytes(attachment_id);
    let mut caches = state.file_caches.write();
    let cache = caches.get_mut(community_id)?;

    let mut delivered: Vec<ControlPayload> = Vec::new();
    for &idx in requested_chunks {
        match cache.get(attachment_uuid, idx) {
            Ok(Some(ciphertext)) => {
                // plaintext_hash field is filled by the requester after FEK
                // decrypt; we store an all-zero placeholder over the wire.
                // (Hash verification happens against AttachmentOffer.chunk_hashes.)
                delivered.push(ControlPayload::AttachmentChunk {
                    attachment_id,
                    chunk_index: idx,
                    data: ciphertext,
                    plaintext_hash: [0u8; 32],
                });
            }
            Ok(None) => {}
            Err(e) => {
                tracing::debug!(
                    community = %community_id,
                    chunk = idx,
                    error = %e,
                    "cache.get failed serving attachment chunk"
                );
            }
        }
    }
    drop(caches);

    if delivered.is_empty() {
        return None;
    }
    rekindle_protocol::capnp_envelope::encode_community_envelope(&CommunityEnvelope::Control(
        ControlPayload::MultiAttachmentChunk { chunks: delivered },
    ))
    .ok()
}

// ─── Phase 4: download (consumer side of app_call) ─────────────────────

#[derive(Debug, Clone)]
struct DiscoveredSource {
    pseudonym: String,
    bitmap: AttachmentBitmap,
}

/// Look up the offer for an attachment from the local channel SMPL record.
/// The offer is embedded in a `ChannelEntry::Message` and was either
/// written by our own upload (so the offer is in our local row) or
/// arrived via the message-notification path during gossip + fetch.
async fn fetch_attachment_offer(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    attachment_id_hex: &str,
) -> Result<AttachmentOffer, String> {
    let target_attachment_id = parse_attachment_id_hex(attachment_id_hex)?;
    let channel_key = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.channel_log_keys.get(channel_id).cloned())
            .ok_or_else(|| "channel record key missing".to_string())?
    };
    let record_key = channel_key
        .parse::<veilid_core::RecordKey>()
        .map_err(|e| format!("invalid channel record key: {e}"))?;
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;

    // Scan all member subkeys for any Message entry carrying our offer.
    for subkey in 0u32..255 {
        let Ok(Some(value)) = rc.get_dht_value(record_key.clone(), subkey, false).await else {
            continue;
        };
        let Ok(entries) =
            rekindle_protocol::dht::community::channel_record::decode_channel_entries(value.data())
        else {
            continue;
        };
        for entry in entries {
            if let
                rekindle_protocol::dht::community::channel_record::ChannelRecordEntry::Message(
                    msg,
                ) = entry
            {
                if let Some(offer) = msg.attachment {
                    if offer.attachment_id == target_attachment_id {
                        return Ok(offer);
                    }
                }
            }
        }
    }
    Err(format!(
        "attachment {attachment_id_hex} not found in channel {channel_id}"
    ))
}

/// Scan all member SMPL subkeys for `AttachmentCached` entries that
/// match `attachment_id`, returning each peer's possession bitmap.
async fn discover_sources(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    attachment_id: [u8; 16],
    chunk_count: u32,
) -> Result<Vec<DiscoveredSource>, String> {
    let channel_key = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.channel_log_keys.get(channel_id).cloned())
            .ok_or_else(|| "channel record key missing".to_string())?
    };
    let record_key = channel_key
        .parse::<veilid_core::RecordKey>()
        .map_err(|e| format!("invalid channel record key: {e}"))?;
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    // For each member subkey: keep the highest-lamport AttachmentCached
    // matching attachment_id. (LWW per (peer, attachment_id).)
    use std::collections::HashMap;
    let mut latest: HashMap<String, (u64, AttachmentBitmap)> = HashMap::new();
    for subkey in 0u32..255 {
        let Ok(Some(value)) = rc.get_dht_value(record_key.clone(), subkey, false).await else {
            continue;
        };
        let Ok(entries) =
            rekindle_protocol::dht::community::channel_record::decode_channel_entries(value.data())
        else {
            continue;
        };
        for entry in entries {
            if let
                rekindle_protocol::dht::community::channel_record::ChannelRecordEntry::AttachmentCached(
                    cached,
                ) = entry
            {
                if cached.attachment_id != attachment_id {
                    continue;
                }
                if cached.chunk_count != chunk_count {
                    continue;
                }
                let Some(bitmap) =
                    AttachmentBitmap::from_bytes(cached.chunk_bitmap, chunk_count)
                else {
                    continue;
                };
                let prev = latest
                    .get(&cached.author_pseudonym)
                    .map_or(0, |(l, _)| *l);
                if cached.lamport_ts >= prev {
                    latest.insert(
                        cached.author_pseudonym,
                        (cached.lamport_ts, bitmap),
                    );
                }
            }
        }
    }
    Ok(latest
        .into_iter()
        .map(|(pseudonym, (_, bitmap))| DiscoveredSource { pseudonym, bitmap })
        .collect())
}

fn parse_attachment_id_hex(hex_str: &str) -> Result<[u8; 16], String> {
    hex::decode(hex_str)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| format!("invalid attachment id hex: {hex_str}"))
}

/// Download an attachment by hex id to `save_path`. v1 strategy: ask each
/// discovered source in turn for missing chunks; verify each chunk against
/// the offer's hash list; reassemble + write to disk; advertise full
/// possession via `AttachmentCached`.
pub async fn download_attachment(
    state: &SharedState,
    pool: &crate::db::DbPool,
    community_id: &str,
    channel_id: &str,
    attachment_id_hex: &str,
    save_path: &Path,
) -> Result<(), String> {
    crate::commands::community::require_permission(
        state,
        community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::READ_MESSAGE_HISTORY,
    )?;
    ensure_cache_open(state, community_id)?;

    let offer = fetch_attachment_offer(state, community_id, channel_id, attachment_id_hex).await?;
    verify_merkle_root(&offer).map_err(|e| format!("offer corrupt: {e}"))?;
    let attachment_id = offer.attachment_id;
    let attachment_uuid = Uuid::from_bytes(attachment_id);
    let chunk_count = offer.chunk_count;

    // Load + unwrap FEK using the historical channel MEK that wrapped it.
    let fek = unwrap_fek_for_offer(state, community_id, channel_id, &offer)?;

    // Compute current-bitmap of what we already have locally.
    let initial_bitmap: AttachmentBitmap = {
        let mut caches = state.file_caches.write();
        let cache = caches
            .get_mut(community_id)
            .ok_or_else(|| "file cache not open for community".to_string())?;
        cache
            .bitmap_for(attachment_uuid, chunk_count)
            .map_err(|e| format!("bitmap_for failed: {e}"))?
    };
    let mut have = initial_bitmap;

    let sources =
        discover_sources(state, community_id, channel_id, attachment_id, chunk_count).await?;
    if sources.is_empty() {
        return Err(
            "no peers advertise this attachment — try again when at least one source is online"
                .into(),
        );
    }

    // Walk sources in arrival order; for each, request the chunks they have
    // that we still need.
    for src in &sources {
        let needed: Vec<u32> = src.bitmap.intersect(&inverse(&have));
        if needed.is_empty() {
            continue;
        }
        let response = send_chunk_request(state, community_id, channel_id, attachment_id, &needed, &src.pseudonym)
            .await?;
        for chunk in response {
            // Decrypt then verify against the offer's plaintext SHA-256.
            let plaintext = fek
                .decrypt(&chunk.data)
                .map_err(|e| format!("FEK decrypt chunk {} failed: {e}", chunk.chunk_index))?;
            let expected = offer
                .chunk_hashes
                .get(chunk.chunk_index as usize)
                .ok_or_else(|| {
                    format!(
                        "received chunk {} out of range (count {chunk_count})",
                        chunk.chunk_index
                    )
                })?;
            if let Err(e) = verify_chunk(&plaintext, expected) {
                tracing::warn!(
                    community = %community_id,
                    peer = %src.pseudonym,
                    chunk = chunk.chunk_index,
                    error = %e,
                    "dropping malformed chunk; will re-request"
                );
                continue;
            }
            // Re-encrypt with FEK for cache storage (so we can serve the
            // same wire format back to other peers without re-keying).
            let stored = fek
                .encrypt(&plaintext)
                .map_err(|e| format!("FEK re-encrypt for cache: {e}"))?;
            {
                let mut caches = state.file_caches.write();
                let cache = caches
                    .get_mut(community_id)
                    .ok_or_else(|| "file cache vanished mid-download".to_string())?;
                let pinned_lock = state.pinned_attachments.read();
                let pinned = pinned_lock
                    .get(community_id)
                    .cloned()
                    .unwrap_or_default();
                cache
                    .insert(attachment_uuid, chunk.chunk_index, &stored, &pinned)
                    .map_err(|e| format!("cache insert chunk {}: {e}", chunk.chunk_index))?;
            }
            have.set(chunk.chunk_index);
        }
        if have.is_complete() {
            break;
        }
    }

    if !have.is_complete() {
        return Err(format!(
            "incomplete download: have {}/{chunk_count} chunks",
            have.count()
        ));
    }

    // Reassemble plaintext + write to disk.
    let mut out: Vec<u8> = Vec::with_capacity(usize::try_from(offer.total_size).unwrap_or(0));
    {
        let mut caches = state.file_caches.write();
        let cache = caches
            .get_mut(community_id)
            .ok_or_else(|| "file cache vanished post-download".to_string())?;
        for idx in 0..chunk_count {
            let ciphertext = cache
                .get(attachment_uuid, idx)
                .map_err(|e| format!("cache get {idx}: {e}"))?
                .ok_or_else(|| format!("chunk {idx} missing from cache mid-reassembly"))?;
            let plaintext = fek
                .decrypt(&ciphertext)
                .map_err(|e| format!("FEK decrypt for reassembly {idx}: {e}"))?;
            out.extend_from_slice(&plaintext);
        }
    }
    if let Some(parent) = save_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create save dir: {e}"))?;
    }
    std::fs::write(save_path, &out)
        .map_err(|e| format!("write {}: {e}", save_path.display()))?;

    // Advertise full possession to the swarm.
    let ctx = build_upload_context(state, community_id, channel_id)?;
    write_self_attachment_cached(
        state,
        &ctx,
        attachment_id,
        chunk_count,
        AttachmentBitmap::full(chunk_count),
    )
    .await?;

    // Update SQLite row's local_path so the UI flips to "Open" instead of "Download".
    persist_local_path_for_attachment(
        pool,
        &state_helpers::current_owner_key(state)?,
        channel_id,
        attachment_id_hex,
        save_path,
    )
    .await?;

    Ok(())
}

fn inverse(bm: &AttachmentBitmap) -> AttachmentBitmap {
    let count = bm.chunk_count();
    let mut out = AttachmentBitmap::new(count);
    for i in 0..count {
        if !bm.has(i) {
            out.set(i);
        }
    }
    out
}

fn unwrap_fek_for_offer(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    offer: &AttachmentOffer,
) -> Result<MediaEncryptionKey, String> {
    // Try the historical channel MEK at `fek_mek_generation` first.
    let from_keystore = {
        let app_handle = state_helpers::app_handle(state);
        if let Some(ah) = app_handle {
            use tauri::Manager;
            let keystore: tauri::State<'_, crate::keystore::KeystoreHandle> = ah.state();
            let guard = keystore.lock();
            guard.as_ref().and_then(|ks| {
                crate::keystore::load_channel_mek_generation(
                    ks,
                    community_id,
                    channel_id,
                    offer.fek_mek_generation,
                )
            })
        } else {
            None
        }
    };
    let mek = if let Some(mek) = from_keystore {
        mek
    } else {
        // Fallback: current per-channel cache.
        let cache = state.channel_mek_cache.lock();
        if let Some(mek) =
            cache.get(&(community_id.to_string(), channel_id.to_string()))
        {
            if mek.generation() == offer.fek_mek_generation {
                mek.clone()
            } else {
                drop(cache);
                state
                    .mek_cache
                    .lock()
                    .get(community_id)
                    .filter(|m| m.generation() == offer.fek_mek_generation)
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "channel MEK for generation {} not available",
                            offer.fek_mek_generation
                        )
                    })?
            }
        } else {
            drop(cache);
            state
                .mek_cache
                .lock()
                .get(community_id)
                .filter(|m| m.generation() == offer.fek_mek_generation)
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "channel MEK for generation {} not available",
                        offer.fek_mek_generation
                    )
                })?
        }
    };
    let raw = mek
        .decrypt(&offer.wrapped_fek)
        .map_err(|e| format!("unwrap FEK failed: {e}"))?;
    if raw.len() != 32 {
        return Err(format!("wrapped FEK plaintext is {} bytes (expected 32)", raw.len()));
    }
    let key: [u8; 32] = raw
        .as_slice()
        .try_into()
        .map_err(|_| "FEK length mismatch".to_string())?;
    Ok(MediaEncryptionKey::from_bytes(key, 0))
}

async fn send_chunk_request(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    attachment_id: [u8; 16],
    requested_chunks: &[u32],
    target_pseudonym: &str,
) -> Result<Vec<ChunkResponse>, String> {
    let route_blob = {
        let communities = state.communities.read();
        let community = communities
            .get(community_id)
            .ok_or_else(|| "community not found".to_string())?;
        let online = community
            .gossip
            .as_ref()
            .ok_or_else(|| "no gossip overlay yet".to_string())?;
        online
            .online_members
            .get(target_pseudonym)
            .map(|m| m.route_blob.clone())
            .ok_or_else(|| {
                format!("source peer {target_pseudonym} not online — cannot app_call")
            })?
    };

    let requester_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .ok_or_else(|| "no pseudonym for community".to_string())?
    };

    let payload = CommunityEnvelope::Control(ControlPayload::RequestAttachment {
        channel_id: channel_id.to_string(),
        attachment_id,
        requested_chunks: requested_chunks.to_vec(),
        requester_pseudonym,
    });
    let bytes = rekindle_protocol::capnp_envelope::encode_community_envelope(&payload)
        .map_err(|e| format!("encode RequestAttachment: {e}"))?;

    let api = state_helpers::veilid_api(state).ok_or("Veilid API unavailable")?;
    let route_id = api
        .import_remote_private_route(route_blob)
        .map_err(|e| format!("import route: {e}"))?;
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let reply = rc
        .app_call(veilid_core::Target::RouteId(route_id), bytes)
        .await
        .map_err(|e| format!("app_call failed: {e}"))?;

    let envelope: CommunityEnvelope =
        rekindle_protocol::capnp_envelope::decode_community_envelope(&reply)
            .map_err(|e| format!("decode reply: {e}"))?;
    match envelope {
        CommunityEnvelope::Control(ControlPayload::MultiAttachmentChunk { chunks }) => Ok(chunks
            .into_iter()
            .filter_map(|c| match c {
                ControlPayload::AttachmentChunk {
                    chunk_index, data, ..
                } => Some(ChunkResponse { chunk_index, data }),
                _ => None,
            })
            .collect()),
        _ => Ok(Vec::new()),
    }
}

#[derive(Debug, Clone)]
struct ChunkResponse {
    chunk_index: u32,
    data: Vec<u8>,
}

async fn persist_local_path_for_attachment(
    pool: &crate::db::DbPool,
    owner_key: &str,
    channel_id: &str,
    attachment_id_hex: &str,
    save_path: &Path,
) -> Result<(), String> {
    let owner = owner_key.to_string();
    let chan = channel_id.to_string();
    let attachment_id_hex = attachment_id_hex.to_string();
    let new_path = save_path.display().to_string();
    crate::db_helpers::db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT message_id, attachment_json FROM messages \
             WHERE owner_key = ?1 AND conversation_id = ?2 AND conversation_type = 'channel' \
             AND attachment_json IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![owner, chan], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        for (message_id, json) in rows {
            let Ok(mut record) = serde_json::from_str::<AttachmentRecordJson>(&json) else {
                continue;
            };
            if record.attachment_id != attachment_id_hex {
                continue;
            }
            record.local_path = Some(new_path.clone());
            let updated =
                serde_json::to_string(&record).unwrap_or_else(|_| json.clone());
            conn.execute(
                "UPDATE messages SET attachment_json = ?1 \
                 WHERE owner_key = ?2 AND conversation_id = ?3 AND message_id = ?4",
                rusqlite::params![updated, owner, chan, message_id],
            )?;
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("update attachment_json local_path: {e}"))
}

// ─── Phase 4: pin / unpin command body ─────────────────────────────────

pub async fn set_attachment_pinned(
    state: &SharedState,
    community_id: &str,
    attachment_id_hex: &str,
    pinned: bool,
) -> Result<(), String> {
    crate::commands::community::require_permission(
        state,
        community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::MANAGE_COMMUNITY,
    )?;
    let attachment_id = parse_attachment_id_hex(attachment_id_hex)?;
    let lamport = state_helpers::increment_lamport(state, community_id);
    crate::services::community::write_entry(
        state,
        community_id,
        rekindle_types::governance::GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned,
            lamport,
        },
    )
    .await
}

// ─── Phase 4: progress event for the UI ────────────────────────────────

pub fn emit_attachment_complete(
    app_handle: &AppHandle,
    community_id: &str,
    channel_id: &str,
    attachment_id_hex: &str,
    local_path: &Path,
) {
    let _ = app_handle.emit(
        "community-event",
        crate::channels::CommunityEvent::AttachmentDownloaded {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            attachment_id: attachment_id_hex.to_string(),
            local_path: local_path.display().to_string(),
        },
    );
}
