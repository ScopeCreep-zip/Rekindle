//! Phase 15 — Lost Cargo upload orchestration.
//!
//! Architecture §28.9 — chunk the bytes, FEK-encrypt each chunk, write
//! chunks to the local cache, build an `AttachmentOffer`, queue the
//! `ChannelMessage` carrying it on our channel outbox stream (plan E3.3
//! slice 7: the stream seals it at its outbox position and the author's
//! `HeadAdvanced` tells readers), then advertise full possession via an
//! `AttachmentCached` entry. Returns the new `attachment_id` (hex-encoded
//! 16-byte UUID).
//!
//! Parameterised over `FilesDeps` so the crate never touches
//! `AppState` / `tauri::AppHandle` / `veilid-core` directly. The
//! Host's `HostFiles` supplies the concrete wiring.

use std::path::Path;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_types::permissions;
use uuid::Uuid;

use crate::chunker::{Chunker, CHUNK_SIZE_BYTES, MAX_FILE_SIZE_BYTES};
use crate::deps::FilesDeps;
use crate::error::FilesError;
use crate::manifest::validate_offer;

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

impl AttachmentRecordJson {
    /// The record of `offer`, as the uploader stores it and as every
    /// receiver's ingest stores it from the post's embedded offer;
    /// `local_path` is where the file sits on this device, if anywhere.
    #[must_use]
    pub fn from_offer(
        offer: &rekindle_types::attachment::AttachmentOffer,
        local_path: Option<String>,
    ) -> Self {
        Self {
            attachment_id: hex::encode(offer.attachment_id),
            filename: offer.filename.clone(),
            mime_type: offer.mime_type.clone(),
            total_size: offer.total_size,
            chunk_count: offer.chunk_count,
            local_path,
        }
    }
}

/// Caller-supplied inputs for [`upload_bytes_as_attachment`]. Groups the
/// file payload + addressing + body so the core pipeline stays under the
/// argument-count budget; each caller still names every field at the
/// construction site.
pub struct UploadRequest {
    pub community_id: String,
    pub channel_id: String,
    pub bytes: Vec<u8>,
    pub filename: String,
    pub mime_type: String,
    /// MEK-encrypted body plaintext (empty for plain uploads; JSON
    /// metadata for voice messages).
    pub body_plaintext: Vec<u8>,
    pub flags: u32,
    /// Local source path, persisted into the SQLite attachment row.
    pub local_path: Option<String>,
}

/// Bundle of state computed once per upload — pseudonym, slot keypair,
/// channel record key, current channel MEK. (The crate-side body uses
/// the outer `community_id` arg directly so we don't carry it in the
/// context.)
struct UploadContext {
    sender_pseudonym: String,
    /// The channel's current text key; its generation is stamped on the
    /// FEK wrap and the carrying message.
    text_key: MediaEncryptionKey,
}

fn build_upload_context<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<UploadContext, FilesError> {
    deps.require_permission(community_id, Some(channel_id), permissions::SEND_MESSAGES)?;
    if deps.channel_is_forum(community_id, channel_id) {
        return Err(FilesError::InvalidInput(
            "forum channels accept posts only through thread creation".into(),
        ));
    }
    let sender_pseudonym = deps.my_pseudonym(community_id)?;
    let text_key = crate::keys::current_text_key(deps, community_id, channel_id)?;
    Ok(UploadContext {
        sender_pseudonym,
        text_key,
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
    let bytes = tokio::fs::read(file_path)
        .await
        .map_err(|e| FilesError::Io {
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
        UploadRequest {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            bytes,
            filename,
            mime_type,
            body_plaintext: Vec::new(),
            flags: 0,
            local_path: Some(file_path.display().to_string()),
        },
    )
    .await
}

/// Core upload helper: chunk + FEK-encrypt + cache `bytes`, build the
/// `AttachmentOffer`, queue the carrying `ChannelMessage` on our outbox
/// stream, announce possession via `AttachmentCached`. Returns the new
/// attachment_id (hex). Used by both regular file uploads
/// ([`upload_file`]) and voice messages ([`send_voice_message_bytes`]).
pub async fn upload_bytes_as_attachment<D: FilesDeps>(
    deps: &D,
    request: UploadRequest,
) -> Result<String, FilesError> {
    let UploadRequest {
        community_id,
        channel_id,
        bytes,
        filename,
        mime_type,
        body_plaintext,
        flags,
        local_path,
    } = request;
    let community_id = community_id.as_str();
    let channel_id = channel_id.as_str();
    let body_plaintext = body_plaintext.as_slice();
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
    deps.enforce_slowmode(community_id, channel_id, timestamp_now(deps))?;
    deps.ensure_cache_open(community_id)?;

    // Per-file FEK (plan §1.J1): fresh AES-256 key, never reused.
    let fek = MediaEncryptionKey::generate_with(0, deps.rng().as_ref());

    // Chunk the plaintext + compute chunk_hashes + Merkle root.
    let chunked = Chunker::chunk(&bytes, *random_uuid(deps).as_bytes())
        .map_err(|e| FilesError::InvalidManifest(format!("chunker: {e}")))?;
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

    // Wrap FEK under the channel's text key at upload time (plan §1.J1).
    let mek_generation = ctx.text_key.generation();
    let wrapped_fek = ctx
        .text_key
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
        fek_mek_generation: mek_generation,
    };
    validate_offer(&offer).map_err(|e| FilesError::OfferInvalid(format!("self-check: {e}")))?;

    // The carrying message is sealed when our channel_post stream drains
    // it onto our outbox (plan E3.3 slice 7); its row is the echo.
    let timestamp_ms = timestamp_now(deps);
    let message_id = format!("msg_{}", random_uuid(deps).simple());

    // Persist a SQLite row.
    let owner_key = deps.owner_key()?;
    let attachment_json =
        serde_json::to_string(&AttachmentRecordJson::from_offer(&offer, local_path))
            .map_err(|e| FilesError::Db(format!("serialize attachment_json: {e}")))?;
    let body_for_db = if flags & rekindle_types::channel::flags::VOICE_MESSAGE != 0 {
        // Voice-message body is JSON metadata: persist it as-is so the local
        // SQLite read path can decode without a separate fetch.
        String::from_utf8_lossy(body_plaintext).to_string()
    } else {
        String::new()
    };
    deps.insert_channel_message_full(crate::deps::InsertChannelMessage {
        owner_key: &owner_key,
        channel_id,
        sender_key: &ctx.sender_pseudonym,
        message_id: &message_id,
        timestamp_ms,
        mek_generation,
        attachment_json: &attachment_json,
        flags,
        body: &body_for_db,
    })
    .await?;

    crate::post::queue_attachment_post(deps, community_id, channel_id, &message_id, &offer, flags)
        .await?;

    // Architecture §28.7 — record this send for the slowmode window so
    // the next call to enforce_slowmode (from any send path) sees the
    // updated timestamp.
    deps.persist_slowmode_state(community_id, channel_id, timestamp_now(deps));

    // No `AttachmentCached` of ours: the offer on our message says we
    // hold every chunk (`discover_sources`).
    Ok(hex::encode(attachment_id))
}

/// Architecture §16.4 — a voice message is a `ChannelEntry::Message`
/// with `flags |= VOICE_MESSAGE` carrying a single `audio/ogg` Lost
/// Cargo attachment: the Ogg Opus file the Host encoded (plan E5.1). It
/// carries no duration or waveform: each receiver decodes them from the
/// audio, so a sender cannot claim them (prod research batch 7 V-2).
pub async fn send_voice_message_bytes<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    opus_bytes: Vec<u8>,
) -> Result<String, FilesError> {
    deps.require_permission(
        community_id,
        Some(channel_id),
        permissions::SEND_VOICE_MESSAGES,
    )?;
    if opus_bytes.is_empty() {
        return Err(FilesError::InvalidInput(
            "voice message has empty audio bytes".into(),
        ));
    }

    upload_bytes_as_attachment(
        deps,
        UploadRequest {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            bytes: opus_bytes,
            filename: format!("voice-{}.ogg", random_uuid(deps).simple()),
            mime_type: "audio/ogg".to_string(),
            body_plaintext: Vec::new(),
            flags: rekindle_types::channel::flags::VOICE_MESSAGE,
            local_path: None,
        },
    )
    .await
}

/// Now, in Unix milliseconds, from the host's clock port (plan C9.2).
fn timestamp_now<D: FilesDeps>(deps: &D) -> i64 {
    i64::try_from(deps.clock().now_ms()).unwrap_or(i64::MAX)
}

/// A random (v4-shaped) id from the host's RNG port (plan C9.2).
fn random_uuid<D: FilesDeps>(deps: &D) -> Uuid {
    uuid::Builder::from_random_bytes(rekindle_utils::random::id_bytes_16_from(
        deps.rng().as_ref(),
    ))
    .into_uuid()
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod tests;
