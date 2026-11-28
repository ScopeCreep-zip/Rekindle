//! Pure thread policy: archive rules, defaults, the message view.

use crate::error::ChannelError;

pub fn validate_auto_archive_seconds(secs: u64) -> Result<u64, ChannelError> {
    if matches!(secs, 3600 | 86_400 | 259_200 | 604_800) {
        Ok(secs)
    } else {
        Err(ChannelError::InvalidId(format!(
            "auto_archive_seconds must be 3600, 86400, 259200, or 604800 — got {secs}"
        )))
    }
}

#[must_use]
pub fn default_auto_archive_seconds(thread_type: &str) -> u64 {
    match thread_type {
        "forum_post" => 604_800,
        "announcement" => 259_200,
        _ => 86_400,
    }
}

/// Architecture §4.6 — a thread is archived when a manual archive stands
/// (no `ThreadUnarchived` after it and no reply naming it, see
/// `ChannelMessage::revives`), OR when it has been idle past its
/// auto-archive window (a local UI rule, T3).
#[must_use]
pub fn is_thread_archived(
    manually_archived: bool,
    last_activity_secs: u64,
    auto_archive_seconds: u64,
    now_secs: u64,
) -> bool {
    let auto_archived = last_activity_secs > 0
        && last_activity_secs.saturating_add(auto_archive_seconds) < now_secs;
    manually_archived || auto_archived
}

/// One thread message decrypted and ready for adapter-side display
/// assembly. Crate-side counterpart of src-tauri `Message` — adapter
/// wraps these into the full Message DTO.
#[derive(Debug, Clone)]
pub struct ThreadMessageView {
    pub admission: Option<rekindle_types::dto::channel_message::PostAdmission>,
    pub sender_pseudonym: String,
    pub body: String,
    /// The body did not open under its key generation at its position
    /// (generation not held yet, or not authentic there).
    pub decryption_failed: bool,
    pub timestamp_ms: u64,
    pub is_own: bool,
    pub send_state: rekindle_types::message::SendState,
    pub server_message_id: Option<String>,
    /// A sealed reply names its key generation. Pending replies are unsealed.
    pub mek_generation: Option<u64>,
    /// Its author's stream seq in the thread (the body's position).
    pub stream_seq: u64,
}

#[cfg(test)]
mod tests;
