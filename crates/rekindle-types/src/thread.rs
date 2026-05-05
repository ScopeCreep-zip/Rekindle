//! Architecture §22 — thread metadata broadcast over the
//! `ThreadCreated` control envelope. Wire shape preserved from the
//! pre-migration JSON-over-envelope form.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadInfo {
    /// 16-byte UUID hex (`thr_<32 hex>`).
    pub id: String,
    /// 16-byte UUID hex of the parent channel.
    pub channel_id: String,
    pub name: String,
    /// Architecture §22 line 2670 — the originating message ID.
    pub starter_message_id: String,
    /// Hex-encoded creator pseudonym.
    pub creator_pseudonym: String,
    /// Forum-channel tag this thread is filed under, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forum_tag: Option<String>,
    pub created_at: u64,
    pub archived: bool,
    /// Architecture §22 line 2675 — auto-archive timeout in seconds
    /// (typically 60m / 24h / 3d / 1w).
    pub auto_archive_seconds: u32,
    pub last_message_at: u64,
    pub message_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_info_roundtrip() {
        let t = ThreadInfo {
            id: "thr_abcdef".into(),
            channel_id: "ch_123".into(),
            name: "design".into(),
            starter_message_id: "msg_xyz".into(),
            creator_pseudonym: "abcd".into(),
            forum_tag: Some("question".into()),
            created_at: 100,
            archived: false,
            auto_archive_seconds: 86_400,
            last_message_at: 200,
            message_count: 7,
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: ThreadInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }
}
