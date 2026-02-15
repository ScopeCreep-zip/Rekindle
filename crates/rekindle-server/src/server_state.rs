use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use parking_lot::RwLock;
use rusqlite::Connection;
use veilid_core::{RoutingContext, VeilidAPI};

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::{PermissionOverwrite, RoleDefinition};

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
    /// Channels in this community.
    pub channels: Vec<ServerChannel>,
    /// Role definitions for this community.
    pub roles: Vec<RoleDefinition>,
    /// Hex-encoded pseudonym key of the community creator (inherent full permissions).
    pub creator_pseudonym_hex: String,
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
}

/// A channel in a hosted community.
pub struct ServerChannel {
    /// Unique channel ID.
    pub id: String,
    /// Channel display name.
    pub name: String,
    /// "text" or "voice".
    pub channel_type: String,
    /// Sort order for display.
    pub sort_order: i32,
    /// Per-channel permission overwrites.
    pub permission_overwrites: Vec<PermissionOverwrite>,
}
