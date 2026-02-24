use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use parking_lot::RwLock;
use rusqlite::Connection;
use veilid_core::{RoutingContext, VeilidAPI};

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_crypto::identity::Identity;
use rekindle_protocol::dht::community::{PermissionOverwrite, RoleDefinition};

use crate::automod::RateLimiter;

/// Central state for the community server daemon.
pub struct ServerState {
    /// Veilid API singleton for this server process.
    pub api: VeilidAPI,
    /// Routing context cloned from the API.
    pub routing_context: RoutingContext,
    /// Server's own `SQLite` database.
    pub db: Arc<Mutex<Connection>>,
    /// Hosted communities: `community_id` -> state.
    pub hosted: RwLock<HashMap<String, HostedCommunity>>,
    /// Unix timestamp when the server started.
    pub started_at: u64,
    /// Server's Ed25519 identity (application-level signing + public key).
    pub identity: Identity,
    /// Hex-encoded public key of the server identity.
    pub public_key_hex: String,
    /// Per-(channel, sender) last message timestamp for slowmode enforcement.
    pub slowmode_last_message: RwLock<HashMap<(String, String), i64>>,
    /// Rate limiter for spam detection (auto-moderation).
    pub rate_limiter: RateLimiter,
    /// Active IPC broadcast listeners. Key = pseudonym_key, Value = broadcast sender.
    /// Used to deliver real-time broadcasts to the hosted community owner on the same machine.
    pub broadcast_listeners: RwLock<HashMap<String, tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>,
}

/// State for a single hosted community.
pub struct HostedCommunity {
    /// Unique community identifier.
    pub community_id: String,
    /// DHT record key for this community.
    pub dht_record_key: String,
    /// Keypair that owns the DHT record (hex-encoded).
    pub owner_keypair_hex: String,
    /// Community display name.
    pub name: String,
    /// Community description.
    pub description: String,
    /// Our private route ID for this community (clients send `app_call` to this).
    pub route_id: Option<veilid_core::RouteId>,
    /// Our private route blob (published to DHT subkey 6).
    pub route_blob: Option<Vec<u8>>,
    /// Current MEK (plaintext on server side).
    pub mek: MediaEncryptionKey,
    /// In-memory roster of members.
    pub members: Vec<ServerMember>,
    /// Channel categories for this community.
    pub categories: Vec<ServerCategory>,
    /// Channels in this community.
    pub channels: Vec<ServerChannel>,
    /// Role definitions for this community.
    pub roles: Vec<RoleDefinition>,
    /// Hex-encoded pseudonym key of the community creator (inherent full permissions).
    pub creator_pseudonym_hex: String,
}

impl HostedCommunity {
    /// Find a member by pseudonym key.
    pub fn find_member(&self, pseudonym: &str) -> Option<&ServerMember> {
        self.members
            .iter()
            .find(|m| m.pseudonym_key_hex == pseudonym)
    }

    /// Find a mutable member by pseudonym key.
    pub fn find_member_mut(&mut self, pseudonym: &str) -> Option<&mut ServerMember> {
        self.members
            .iter_mut()
            .find(|m| m.pseudonym_key_hex == pseudonym)
    }

    /// Check if a pseudonym is a member.
    pub fn is_member(&self, pseudonym: &str) -> bool {
        self.members
            .iter()
            .any(|m| m.pseudonym_key_hex == pseudonym)
    }

    /// Check if a pseudonym is the community creator.
    pub fn is_creator(&self, pseudonym: &str) -> bool {
        !self.creator_pseudonym_hex.is_empty() && self.creator_pseudonym_hex == pseudonym
    }

    /// Find a channel by ID.
    pub fn find_channel(&self, channel_id: &str) -> Option<&ServerChannel> {
        self.channels.iter().find(|c| c.id == channel_id)
    }

    /// Find a mutable channel by ID.
    pub fn find_channel_mut(&mut self, channel_id: &str) -> Option<&mut ServerChannel> {
        self.channels.iter_mut().find(|c| c.id == channel_id)
    }

    /// Find a category by ID.
    pub fn find_category(&self, category_id: &str) -> Option<&ServerCategory> {
        self.categories.iter().find(|c| c.id == category_id)
    }
}

/// A member in the server's roster.
pub struct ServerMember {
    /// Hex-encoded pseudonym Ed25519 public key.
    pub pseudonym_key_hex: String,
    /// Display name (plaintext for now).
    pub display_name: String,
    /// Role IDs assigned to this member.
    pub role_ids: Vec<u32>,
    /// When the member joined (unix timestamp ms).
    pub joined_at: i64,
    /// The member's private route blob (for broadcasting messages).
    pub route_blob: Option<Vec<u8>>,
    /// If set, the member is timed out until this unix timestamp (seconds).
    pub timeout_until: Option<u64>,
    /// In-memory presence status (not persisted — resets to "offline" on restart).
    pub online_status: String,
}

/// A channel category (collapsible group header).
pub struct ServerCategory {
    /// Unique category ID.
    pub id: String,
    /// Category display name.
    pub name: String,
    /// Sort order for display.
    pub sort_order: i32,
}

/// A channel in a hosted community.
pub struct ServerChannel {
    /// Unique channel ID.
    pub id: String,
    /// Channel display name.
    pub name: String,
    /// "text", "voice", or "announcement".
    pub channel_type: String,
    /// Sort order for display.
    pub sort_order: i32,
    /// Per-channel permission overwrites.
    pub permission_overwrites: Vec<PermissionOverwrite>,
    /// Optional parent category ID.
    pub category_id: Option<String>,
    /// Channel topic / description.
    pub topic: String,
    /// Slowmode delay in seconds (0 = disabled).
    pub slowmode_seconds: u32,
}
