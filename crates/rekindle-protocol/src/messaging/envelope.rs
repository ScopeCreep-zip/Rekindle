use serde::{Deserialize, Serialize};

/// Wire format envelope wrapping all messages sent over Veilid.
///
/// The payload is E2E encrypted (Signal Protocol for DMs, MEK for channels).
/// This envelope provides sender identification and integrity verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEnvelope {
    /// Sender's Ed25519 public key (32 bytes).
    pub sender_key: Vec<u8>,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// Unique message nonce (for deduplication and ordering).
    pub nonce: Vec<u8>,
    /// Encrypted payload (ciphertext).
    pub payload: Vec<u8>,
    /// Ed25519 signature over (timestamp || nonce || payload).
    pub signature: Vec<u8>,
}

/// The type of message contained in the envelope payload (after decryption).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MessagePayload {
    /// Direct 1:1 chat message.
    DirectMessage {
        body: String,
        reply_to: Option<Vec<u8>>,
    },
    /// Channel message (community text channel).
    ChannelMessage {
        channel_id: String,
        body: String,
        reply_to: Option<Vec<u8>>,
    },
    /// Typing indicator.
    TypingIndicator { typing: bool },
    /// Friend request.
    FriendRequest {
        display_name: String,
        message: String,
        prekey_bundle: Vec<u8>,
    },
    /// Friend request acceptance.
    FriendAccept { prekey_bundle: Vec<u8> },
    /// Friend request rejection.
    FriendReject,
    /// Presence update (status, game info).
    PresenceUpdate {
        status: u8,
        game_info: Option<GameInfo>,
    },
}

/// Game information for rich presence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameInfo {
    pub game_id: u32,
    pub game_name: String,
    pub server_info: Option<String>,
    pub elapsed_seconds: u32,
}
