//! Phase 15 — Lost Cargo upload orchestration.
//!
//! Architecture §28.9 — chunk the bytes, FEK-encrypt each chunk, write
//! chunks to the local cache, build an `AttachmentOffer`, embed it in
//! a `ChannelMessage` written to the channel SMPL record, then
//! advertise full possession via an `AttachmentCached` entry and
//! gossip a `MessageNotification`. Returns the new `attachment_id`
//! (hex-encoded 16-byte UUID).
//!
//! Parameterised over `FilesDeps` so the crate never touches
//! `AppState` / `tauri::AppHandle` / `veilid-core` directly. The
//! src-tauri `FilesAdapter` supplies the concrete wiring.

use std::path::Path;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::channel_record::{
    ChannelAttachmentCached, ChannelMessage, CHANNEL_OWNER_SUBKEY_COUNT,
};
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use uuid::Uuid;

use crate::chunker::{Chunker, CHUNK_SIZE_BYTES, MAX_FILE_SIZE_BYTES};
use crate::deps::FilesDeps;
use crate::error::FilesError;
use crate::manifest::validate_offer;
use rekindle_types::attachment::AttachmentBitmap;

/// JSON payload stored in `messages.attachment_json` so the local UI can
/// render the file metadata without re-fetching the offer from the DHT.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
/// channel record key, current channel MEK. (The crate-side body uses
/// the outer `community_id` arg directly so we don't carry it in the
/// context.)
struct UploadContext {
    channel_key: String,
    slot_keypair: String,
    slot_index: u32,
    sender_pseudonym: String,
    mek_generation: u64,
    channel_mek: MediaEncryptionKey,
}

fn build_upload_context<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<UploadContext, FilesError> {
    deps.require_permission(community_id, Permissions::SEND_MESSAGES)?;
    if deps.channel_is_forum(community_id, channel_id) {
        return Err(FilesError::InvalidInput(
            "forum channels accept posts only through thread creation".into(),
        ));
    }
    let channel_key = deps.channel_log_key(community_id, channel_id)?;
    let slot_keypair = deps.slot_keypair(community_id)?;
    let slot_index = deps.my_subkey_index(community_id)?;
    let sender_pseudonym = deps.my_pseudonym(community_id)?;
    let mek_generation = deps.mek_generation(community_id)?;
    let channel_mek = deps.channel_mek(community_id, channel_id)?;
    Ok(UploadContext {
        channel_key,
        slot_keypair,
        slot_index,
        sender_pseudonym,
        mek_generation,
        channel_mek,
    })
}

/// MIME-type guess from filename extension. Mirrors src-tauri's prior
/// `guess_mime_type`. Falls back to `application/octet-stream`.
#[must_use]
pub fn guess_mime_type(path: &Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
    {
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

/// Read a file from disk + delegate to [`upload_bytes_as_attachment`].
pub async fn upload_file<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    file_path: &Path,
) -> Result<String, FilesError> {
    let bytes = std::fs::read(file_path).map_err(|e| FilesError::Io {
        path: file_path.display().to_string(),
        source: e,
    })?;
    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let mime_type = guess_mime_type(file_path);
    upload_bytes_as_attachment(
        deps,
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
/// `AttachmentOffer`, write the carrying `ChannelMessage`, announce
/// possession via `AttachmentCached`, gossip notify. Returns the new
/// attachment_id (hex). Used by both regular file uploads
/// ([`upload_file`]) and voice messages ([`send_voice_message_bytes`]).
#[allow(clippy::too_many_arguments, reason = "core upload pipeline — bundling args would just push the field-set into a single-use struct")]
pub async fn upload_bytes_as_attachment<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    bytes: Vec<u8>,
    filename: String,
    mime_type: String,
    body_plaintext: &[u8],
    flags: u32,
    local_path: Option<String>,
) -> Result<String, FilesError> {
    let total_size = bytes.len() as u64;
    if total_size > MAX_FILE_SIZE_BYTES {
        return Err(FilesError::FileTooLarge {
            actual: total_size,
            max: MAX_FILE_SIZE_BYTES,
        });
    }

    let ctx = build_upload_context(deps, community_id, channel_id)?;
    // Architecture §28.7 — file uploads (and voice messages, which call
    // through here) write a ChannelMessage carrying the attachment, so
    // the slowmode gate applies just like a plain text send.
    deps.enforce_slowmode(community_id, channel_id, timestamp_now())?;
    deps.ensure_cache_open(community_id)?;

    // Per-file FEK (plan §1.J1): fresh AES-256 key, never reused.
    let fek = MediaEncryptionKey::generate(0);

    // Chunk the plaintext + compute chunk_hashes + Merkle root.
    let chunked =
        Chunker::chunk(&bytes).map_err(|e| FilesError::InvalidManifest(format!("chunker: {e}")))?;
    let attachment_id = chunked.attachment_id;
    let attachment_uuid = Uuid::from_bytes(attachment_id);
    let chunk_count = u32::try_from(chunked.chunks.len())
        .map_err(|_| FilesError::InvalidInput("chunk count exceeds u32::MAX".into()))?;

    // Encrypt + cache each chunk.
    deps.with_cache_mut(community_id, &mut |cache, pinned| {
        for (idx, chunk) in chunked.chunks.iter().enumerate() {
            let ciphertext = fek
                .encrypt(chunk)
                .map_err(|e| FilesError::Encrypt(format!("FEK chunk encrypt at {idx}: {e}")))?;
            let chunk_idx = u32::try_from(idx)
                .map_err(|_| FilesError::InvalidInput("chunk index exceeds u32::MAX".into()))?;
            cache
                .insert(attachment_uuid, chunk_idx, &ciphertext, pinned)
                .map_err(|e| FilesError::Db(format!("cache insert at {chunk_idx}: {e}")))?;
        }
        Ok(())
    })?;

    // Wrap FEK under the channel MEK at upload time (plan §1.J1).
    let wrapped_fek = ctx
        .channel_mek
        .encrypt(fek.as_bytes())
        .map_err(|e| FilesError::Encrypt(format!("wrap FEK: {e}")))?;

    let offer = rekindle_types::attachment::AttachmentOffer {
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
    validate_offer(&offer).map_err(|e| FilesError::OfferInvalid(format!("self-check: {e}")))?;

    // Carrying ChannelMessage. The body holds whatever the caller asked
    // (empty for plain uploads; JSON metadata for voice messages).
    let timestamp_ms = timestamp_now();
    let message_id = format!("msg_{}", Uuid::new_v4().simple());
    let lamport_ts = deps.increment_lamport(community_id);
    let sequence = deps.next_channel_sequence(community_id, channel_id);
    let body_ciphertext = ctx
        .channel_mek
        .encrypt(body_plaintext)
        .map_err(|e| FilesError::Encrypt(format!("MEK encrypt body: {e}")))?;

    // Architecture §28.5 — caption mentions go into the cleartext
    // envelope. Voice messages and file uploads can still ping people
    // via the body text. The MENTION_EVERYONE/MENTION_HERE flag bits
    // are OR'd into whatever flags the caller already passed (e.g.
    // VOICE_MESSAGE).
    let body_text = String::from_utf8_lossy(body_plaintext);
    let (mentioned_pseudonyms, mentioned_roles, mention_flags) =
        deps.resolve_outbound_mentions(community_id, &ctx.sender_pseudonym, &body_text);

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
    let owner_key = deps.owner_key()?;
    let attachment_json = serde_json::to_string(&AttachmentRecordJson {
        attachment_id: hex::encode(attachment_id),
        filename: filename.clone(),
        mime_type: mime_type.clone(),
        total_size,
        chunk_count,
        local_path,
    })
    .map_err(|e| FilesError::Db(format!("serialize attachment_json: {e}")))?;
    let body_for_db = if flags & rekindle_types::channel::flags::VOICE_MESSAGE != 0 {
        // Voice-message body is JSON metadata: persist it as-is so the local
        // SQLite read path can decode without a separate fetch.
        String::from_utf8_lossy(body_plaintext).to_string()
    } else {
        String::new()
    };
    deps.insert_channel_message_full(
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
    deps.write_channel_message_to_smpl(
        community_id,
        &ctx.channel_key,
        ctx.slot_index,
        &ctx.slot_keypair,
        &channel_msg,
    )
    .await?;

    // Architecture §28.7 — record this send for the slowmode window so
    // the next call to enforce_slowmode (from any send path) sees the
    // updated timestamp.
    deps.persist_slowmode_state(community_id, channel_id, timestamp_now());

    // Announce full possession via AttachmentCached entry on our subkey.
    let cached_lamport = deps.increment_lamport(community_id);
    let cached = ChannelAttachmentCached {
        attachment_id,
        chunk_bitmap: AttachmentBitmap::full(chunk_count).as_bytes().to_vec(),
        chunk_count,
        author_pseudonym: ctx.sender_pseudonym.clone(),
        lamport_ts: cached_lamport,
    };
    deps.write_attachment_cached_to_smpl(
        community_id,
        &ctx.channel_key,
        ctx.slot_index,
        &ctx.slot_keypair,
        &cached,
    )
    .await?;

    // Gossip a MessageNotification so peers fast-fetch the SMPL write.
    let notification = CommunityEnvelope::MessageNotification {
        channel_id: channel_id.to_string(),
        message_id: message_id.clone(),
        author_pseudonym: ctx.sender_pseudonym.clone(),
        subkey_index: u32::from(CHANNEL_OWNER_SUBKEY_COUNT) + ctx.slot_index,
        lamport_ts,
        sequence,
        content_hash: blake3::hash(&channel_msg.ciphertext).to_hex().to_string(),
        timestamp: channel_msg.timestamp,
    };
    deps.send_to_mesh(community_id, &notification)?;

    Ok(hex::encode(attachment_id))
}

/// Architecture §16.4 — a voice message is a `ChannelEntry::Message`
/// with `flags |= VOICE_MESSAGE` carrying a single `audio/ogg` Lost
/// Cargo attachment plus waveform + duration metadata. The metadata
/// travels inside the (MEK-encrypted) body as JSON so it propagates
/// with the message without changing the AttachmentOffer wire format.
pub async fn send_voice_message_bytes<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    opus_bytes: Vec<u8>,
    duration_ms: u32,
    waveform: Vec<u8>,
) -> Result<String, FilesError> {
    deps.require_permission(community_id, Permissions::SEND_VOICE_MESSAGES)?;
    if duration_ms == 0 || duration_ms > 5 * 60 * 1000 {
        return Err(FilesError::InvalidInput(format!(
            "voice message duration {duration_ms}ms outside 1ms..=5min"
        )));
    }
    if waveform.len() > 256 {
        return Err(FilesError::InvalidInput(format!(
            "waveform has {} peaks — max 256",
            waveform.len()
        )));
    }
    if opus_bytes.is_empty() {
        return Err(FilesError::InvalidInput(
            "voice message has empty audio bytes".into(),
        ));
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
    .map_err(|e| FilesError::Db(format!("serialize voice message body: {e}")))?;

    upload_bytes_as_attachment(
        deps,
        community_id,
        channel_id,
        opus_bytes,
        format!("voice-{}.ogg", Uuid::new_v4().simple()),
        "audio/ogg".to_string(),
        &body,
        rekindle_types::channel::flags::VOICE_MESSAGE,
        None,
    )
    .await
}

fn timestamp_now() -> i64 {
    i64::try_from(rekindle_utils::timestamp_ms()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_mock::MockDeps;

    #[tokio::test]
    async fn happy_path_writes_message_attachment_cached_and_persists_row() {
        let deps = MockDeps::new("c1", "ch1").with_mek(7, [42u8; 32]);
        let bytes = b"hello world payload".to_vec();
        let result = upload_bytes_as_attachment(
            &deps,
            "c1",
            "ch1",
            bytes,
            "hello.txt".into(),
            "text/plain".into(),
            b"",
            0,
            Some("/tmp/hello.txt".into()),
        )
        .await;
        assert!(result.is_ok(), "happy path should succeed");
        let attachment_id_hex = result.unwrap();
        assert_eq!(attachment_id_hex.len(), 32, "16-byte UUID hex");

        let calls = deps.calls.lock();
        assert_eq!(calls.channel_messages_written.len(), 1, "one SMPL message");
        assert_eq!(calls.attachment_cacheds_written.len(), 1, "one AttachmentCached");
        assert_eq!(calls.channel_messages_persisted.len(), 1, "one SQLite row");
        assert_eq!(
            calls.slowmode_persists.len(),
            1,
            "slowmode timestamp persisted"
        );
    }

    #[tokio::test]
    async fn oversize_file_rejected() {
        let deps = MockDeps::new("c1", "ch1").with_mek(1, [0u8; 32]);
        // MAX_FILE_SIZE_BYTES = 28*1024*1000 ≈ 28 MB. Build a 1-byte-larger one.
        let bytes = vec![0u8; (MAX_FILE_SIZE_BYTES + 1) as usize];
        let err = upload_bytes_as_attachment(
            &deps,
            "c1",
            "ch1",
            bytes,
            "big.bin".into(),
            "application/octet-stream".into(),
            b"",
            0,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, FilesError::FileTooLarge { .. }));
    }

    #[tokio::test]
    async fn forum_channel_rejected() {
        let mut deps = MockDeps::new("c1", "ch1").with_mek(1, [0u8; 32]);
        deps.forum_channel = true;
        let err = upload_bytes_as_attachment(
            &deps,
            "c1",
            "ch1",
            b"x".to_vec(),
            "f.txt".into(),
            "text/plain".into(),
            b"",
            0,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, FilesError::InvalidInput(msg) if msg.contains("forum")));
    }

    #[tokio::test]
    async fn permission_denied_returns_error() {
        let mut deps = MockDeps::new("c1", "ch1").with_mek(1, [0u8; 32]);
        deps.permission_pass = false;
        let err = upload_bytes_as_attachment(
            &deps,
            "c1",
            "ch1",
            b"x".to_vec(),
            "f.txt".into(),
            "text/plain".into(),
            b"",
            0,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, FilesError::PermissionDenied(_)));
    }

    #[tokio::test]
    async fn slowmode_blocks_upload() {
        let mut deps = MockDeps::new("c1", "ch1").with_mek(1, [0u8; 32]);
        deps.slowmode_pass = false;
        let err = upload_bytes_as_attachment(
            &deps,
            "c1",
            "ch1",
            b"x".to_vec(),
            "f.txt".into(),
            "text/plain".into(),
            b"",
            0,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, FilesError::Slowmode(_)));
    }

    #[test]
    fn guess_mime_type_matches_extensions() {
        use std::path::PathBuf;
        assert_eq!(guess_mime_type(&PathBuf::from("a.png")), "image/png");
        assert_eq!(guess_mime_type(&PathBuf::from("a.OGG")), "audio/ogg");
        assert_eq!(guess_mime_type(&PathBuf::from("a.unknown")), "application/octet-stream");
        assert_eq!(guess_mime_type(&PathBuf::from("noext")), "application/octet-stream");
    }
}
