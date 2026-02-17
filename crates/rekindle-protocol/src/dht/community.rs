use crate::capnp_codec;
use crate::dht::DHTManager;
use crate::error::ProtocolError;
use serde::{Deserialize, Serialize};

// Community record subkey layout (SMPL schema, multi-writer).
pub const SUBKEY_METADATA: u32 = 0;
pub const SUBKEY_CHANNELS: u32 = 1;
pub const SUBKEY_MEMBERS: u32 = 2;
pub const SUBKEY_ROLES: u32 = 3;
pub const SUBKEY_INVITES: u32 = 4;
pub const SUBKEY_MEK: u32 = 5;
pub const SUBKEY_SERVER_ROUTE: u32 = 6;

pub const COMMUNITY_SUBKEY_COUNT: u32 = 7;

/// The @everyone role always has ID 0.
pub const ROLE_EVERYONE_ID: u32 = 0;

/// Community metadata stored in subkey 0.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityMetadata {
    pub name: String,
    pub description: Option<String>,
    pub icon_hash: Option<String>,
    pub created_at: u64,
    pub owner_key: String,
    /// Timestamp of the last DHT keepalive refresh (seconds since epoch).
    /// Updated each keepalive cycle so the value actually changes, forcing a DHT write.
    #[serde(default)]
    pub last_refreshed: u64,
}

/// A channel entry stored in the channel list (subkey 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelEntry {
    pub id: String,
    pub name: String,
    pub channel_type: String, // "text" or "voice"
    pub sort_order: u16,
    pub latest_message_key: Option<String>,
    #[serde(default)]
    pub permission_overwrites: Vec<PermissionOverwrite>,
}

/// A member entry stored in the member list (subkey 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberEntry {
    pub pseudonym_key: String,
    /// Legacy single-role field. Kept for migration compatibility.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub role: Option<String>,
    /// New multi-role system: list of role IDs the member has.
    #[serde(default)]
    pub role_ids: Vec<u32>,
    pub joined_at: u64,
    /// If set, the member is timed out until this unix timestamp (seconds).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub timeout_until: Option<u64>,
}

/// A role definition stored in the roles list (subkey 3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleDefinition {
    pub id: u32,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    /// Role hierarchy position. Higher = more authority.
    pub position: i32,
    /// Whether to display this role separately in the member list.
    #[serde(default)]
    pub hoist: bool,
    /// Whether this role can be @mentioned by anyone.
    #[serde(default)]
    pub mentionable: bool,
}

/// Permission overwrite for a channel, targeting either a role or a specific member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionOverwrite {
    pub target_type: OverwriteType,
    /// Role ID (as string) or member pseudonym key.
    pub target_id: String,
    pub allow: u64,
    pub deny: u64,
}

/// Whether a permission overwrite targets a role or a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverwriteType {
    Role,
    Member,
}

/// Create a new community DHT record.
pub async fn create_community(
    dht: &DHTManager,
    metadata: &CommunityMetadata,
) -> Result<String, ProtocolError> {
    // TODO: Use DHTSchema::SMPL for multi-writer when veilid-core is available
    let (key, _owner_keypair) = dht.create_record(COMMUNITY_SUBKEY_COUNT).await?;

    let meta_bytes = capnp_codec::community::encode_community(metadata, &[], &[]);
    dht.set_value(&key, SUBKEY_METADATA, meta_bytes).await?;

    // Initialize empty channel list
    let ch_bytes = capnp_codec::community::encode_channels(&[]);
    dht.set_value(&key, SUBKEY_CHANNELS, ch_bytes).await?;

    // Initialize member list with owner (assign owner role + @everyone)
    let members = vec![MemberEntry {
        pseudonym_key: metadata.owner_key.clone(),
        role: None,
        role_ids: vec![ROLE_EVERYONE_ID, 4], // @everyone + Owner role
        joined_at: metadata.created_at,
        timeout_until: None,
    }];
    let mem_bytes = capnp_codec::community::encode_members(&members)?;
    dht.set_value(&key, SUBKEY_MEMBERS, mem_bytes).await?;

    tracing::info!(key = %key, name = %metadata.name, "community record created");
    Ok(key)
}

/// Read community metadata from DHT.
pub async fn read_metadata(
    dht: &DHTManager,
    key: &str,
) -> Result<Option<CommunityMetadata>, ProtocolError> {
    match dht.get_value(key, SUBKEY_METADATA).await? {
        Some(data) => {
            let (meta, _, _) = capnp_codec::community::decode_community(&data, "")?;
            Ok(Some(meta))
        }
        None => Ok(None),
    }
}

/// Read channel list from DHT.
pub async fn read_channels(
    dht: &DHTManager,
    key: &str,
) -> Result<Vec<ChannelEntry>, ProtocolError> {
    match dht.get_value(key, SUBKEY_CHANNELS).await? {
        Some(data) => {
            let (_, channels, _) = capnp_codec::community::decode_community(&data, "")?;
            Ok(channels)
        }
        None => Ok(vec![]),
    }
}

/// Read member list from DHT.
pub async fn read_members(
    dht: &DHTManager,
    key: &str,
) -> Result<Vec<MemberEntry>, ProtocolError> {
    match dht.get_value(key, SUBKEY_MEMBERS).await? {
        Some(data) => capnp_codec::community::decode_members(&data),
        None => Ok(vec![]),
    }
}

/// Add a channel to the community.
pub async fn add_channel(
    dht: &DHTManager,
    key: &str,
    channel: ChannelEntry,
) -> Result<(), ProtocolError> {
    let mut channels = read_channels(dht, key).await?;
    channels.push(channel);
    let data = capnp_codec::community::encode_channels(&channels);
    dht.set_value(key, SUBKEY_CHANNELS, data).await
}

/// Remove a channel from the community by ID.
pub async fn remove_channel(
    dht: &DHTManager,
    key: &str,
    channel_id: &str,
) -> Result<(), ProtocolError> {
    let mut channels = read_channels(dht, key).await?;
    let before = channels.len();
    channels.retain(|c| c.id != channel_id);
    if channels.len() == before {
        return Err(ProtocolError::DhtError(format!(
            "channel {channel_id} not found"
        )));
    }
    let data = capnp_codec::community::encode_channels(&channels);
    dht.set_value(key, SUBKEY_CHANNELS, data).await
}

/// Add a member to the community.
pub async fn add_member(
    dht: &DHTManager,
    key: &str,
    member: MemberEntry,
) -> Result<(), ProtocolError> {
    let mut members = read_members(dht, key).await?;
    if members.iter().any(|m| m.pseudonym_key == member.pseudonym_key) {
        return Err(ProtocolError::DhtError("member already exists".into()));
    }
    members.push(member);
    let data = capnp_codec::community::encode_members(&members)?;
    dht.set_value(key, SUBKEY_MEMBERS, data).await
}

/// Remove a member from the community by pseudonym key.
pub async fn remove_member(
    dht: &DHTManager,
    key: &str,
    pseudonym_key: &str,
) -> Result<(), ProtocolError> {
    let mut members = read_members(dht, key).await?;
    let before = members.len();
    members.retain(|m| m.pseudonym_key != pseudonym_key);
    if members.len() == before {
        return Err(ProtocolError::PeerNotFound(format!(
            "member {pseudonym_key} not found"
        )));
    }
    let data = capnp_codec::community::encode_members(&members)?;
    dht.set_value(key, SUBKEY_MEMBERS, data).await
}

/// Set a member's role IDs (replaces all role assignments).
pub async fn set_member_roles(
    dht: &DHTManager,
    key: &str,
    pseudonym_key: &str,
    role_ids: Vec<u32>,
) -> Result<(), ProtocolError> {
    let mut members = read_members(dht, key).await?;
    let member = members
        .iter_mut()
        .find(|m| m.pseudonym_key == pseudonym_key)
        .ok_or_else(|| ProtocolError::PeerNotFound(format!("member {pseudonym_key} not found")))?;
    member.role_ids = role_ids;
    member.role = None; // clear legacy field
    let data = capnp_codec::community::encode_members(&members)?;
    dht.set_value(key, SUBKEY_MEMBERS, data).await
}

/// Read role definitions from DHT.
pub async fn read_roles(
    dht: &DHTManager,
    key: &str,
) -> Result<Vec<RoleDefinition>, ProtocolError> {
    match dht.get_value(key, SUBKEY_ROLES).await? {
        Some(data) => {
            let (_, _, roles) = capnp_codec::community::decode_community(&data, "")?;
            Ok(roles)
        }
        None => Ok(vec![]),
    }
}

/// Add a role definition to the community.
pub async fn add_role(
    dht: &DHTManager,
    key: &str,
    role: RoleDefinition,
) -> Result<(), ProtocolError> {
    let mut roles = read_roles(dht, key).await?;
    if roles.iter().any(|r| r.id == role.id) {
        let id = role.id;
        return Err(ProtocolError::DhtError(format!(
            "role with id {id} already exists"
        )));
    }
    roles.push(role);
    let data = capnp_codec::community::encode_roles(&roles);
    dht.set_value(key, SUBKEY_ROLES, data).await
}

/// Remove a role definition by ID.
pub async fn remove_role(
    dht: &DHTManager,
    key: &str,
    role_id: u32,
) -> Result<(), ProtocolError> {
    let mut roles = read_roles(dht, key).await?;
    let before = roles.len();
    roles.retain(|r| r.id != role_id);
    if roles.len() == before {
        return Err(ProtocolError::DhtError(format!(
            "role {role_id} not found"
        )));
    }
    let data = capnp_codec::community::encode_roles(&roles);
    dht.set_value(key, SUBKEY_ROLES, data).await
}

/// Update a role definition (replaces the role with matching ID).
pub async fn update_role(
    dht: &DHTManager,
    key: &str,
    role: RoleDefinition,
) -> Result<(), ProtocolError> {
    let mut roles = read_roles(dht, key).await?;
    let entry = roles
        .iter_mut()
        .find(|r| r.id == role.id)
        .ok_or_else(|| {
            let id = role.id;
            ProtocolError::DhtError(format!("role {id} not found"))
        })?;
    *entry = role;
    let data = capnp_codec::community::encode_roles(&roles);
    dht.set_value(key, SUBKEY_ROLES, data).await
}

/// Permission bit flags for role-based access control (Discord-aligned bit positions).
pub mod permissions {
    // ── General ──
    pub const CREATE_INSTANT_INVITE: u64 = 1 << 0;
    pub const KICK_MEMBERS: u64 = 1 << 1;
    pub const BAN_MEMBERS: u64 = 1 << 2;
    pub const ADMINISTRATOR: u64 = 1 << 3;
    pub const MANAGE_CHANNELS: u64 = 1 << 4;
    pub const MANAGE_COMMUNITY: u64 = 1 << 5;

    // ── Text ──
    pub const ADD_REACTIONS: u64 = 1 << 6;
    pub const VIEW_AUDIT_LOG: u64 = 1 << 7;
    pub const PRIORITY_SPEAKER: u64 = 1 << 8;
    pub const STREAM: u64 = 1 << 9;
    pub const VIEW_CHANNEL: u64 = 1 << 10;
    pub const SEND_MESSAGES: u64 = 1 << 11;
    pub const MANAGE_MESSAGES: u64 = 1 << 13;
    pub const EMBED_LINKS: u64 = 1 << 14;
    pub const ATTACH_FILES: u64 = 1 << 15;
    pub const READ_MESSAGE_HISTORY: u64 = 1 << 16;
    pub const MENTION_EVERYONE: u64 = 1 << 17;
    pub const USE_EXTERNAL_EMOJIS: u64 = 1 << 18;

    // ── Voice ──
    pub const CONNECT: u64 = 1 << 20;
    pub const SPEAK: u64 = 1 << 21;
    pub const MUTE_MEMBERS: u64 = 1 << 22;
    pub const DEAFEN_MEMBERS: u64 = 1 << 23;
    pub const MOVE_MEMBERS: u64 = 1 << 24;
    pub const USE_VAD: u64 = 1 << 25;

    // ── Membership ──
    pub const CHANGE_NICKNAME: u64 = 1 << 26;
    pub const MANAGE_NICKNAMES: u64 = 1 << 27;
    pub const MANAGE_ROLES: u64 = 1 << 28;

    // ── Future ──
    pub const MANAGE_THREADS: u64 = 1 << 34;
    pub const CREATE_PUBLIC_THREADS: u64 = 1 << 35;
    pub const CREATE_PRIVATE_THREADS: u64 = 1 << 36;

    // ── Moderation ──
    pub const MODERATE_MEMBERS: u64 = 1 << 40;

    /// Check if a permission bitmask includes a specific permission.
    /// Returns true immediately if the member has ADMINISTRATOR.
    pub fn has_permission(member_permissions: u64, required: u64) -> bool {
        if member_permissions & ADMINISTRATOR != 0 {
            return true;
        }
        member_permissions & required == required
    }

    /// Check if permissions include ADMINISTRATOR.
    pub fn is_administrator(perms: u64) -> bool {
        perms & ADMINISTRATOR != 0
    }

    /// Default permissions for the @everyone role (id=0).
    pub fn everyone_permissions() -> u64 {
        VIEW_CHANNEL
            | READ_MESSAGE_HISTORY
            | CONNECT
            | SEND_MESSAGES
            | SPEAK
            | ADD_REACTIONS
            | EMBED_LINKS
            | ATTACH_FILES
            | USE_EXTERNAL_EMOJIS
            | USE_VAD
            | CHANGE_NICKNAME
    }

    /// Default permissions for the Member role (id=1).
    pub fn member_permissions() -> u64 {
        everyone_permissions() | CREATE_INSTANT_INVITE
    }

    /// Default permissions for the Moderator role (id=2).
    pub fn moderator_permissions() -> u64 {
        member_permissions()
            | KICK_MEMBERS
            | MANAGE_MESSAGES
            | MUTE_MEMBERS
            | DEAFEN_MEMBERS
            | MODERATE_MEMBERS
    }

    /// Default permissions for the Admin role (id=3).
    pub fn admin_permissions() -> u64 {
        moderator_permissions()
            | MANAGE_CHANNELS
            | MANAGE_ROLES
            | BAN_MEMBERS
            | VIEW_AUDIT_LOG
            | MANAGE_NICKNAMES
            | MANAGE_COMMUNITY
    }

    /// All defined permission bits OR'd together. Use this instead of `u64::MAX`
    /// to avoid integer overflow in `SQLite` (i64) and precision loss in JavaScript (f64).
    /// The highest bit is 40 (`MODERATE_MEMBERS` ≈ 1.1 trillion), well within JS safe
    /// integer range (2^53) and positive i64 range.
    pub fn all_permissions() -> u64 {
        CREATE_INSTANT_INVITE
            | KICK_MEMBERS
            | BAN_MEMBERS
            | ADMINISTRATOR
            | MANAGE_CHANNELS
            | MANAGE_COMMUNITY
            | ADD_REACTIONS
            | VIEW_AUDIT_LOG
            | PRIORITY_SPEAKER
            | STREAM
            | VIEW_CHANNEL
            | SEND_MESSAGES
            | MANAGE_MESSAGES
            | EMBED_LINKS
            | ATTACH_FILES
            | READ_MESSAGE_HISTORY
            | MENTION_EVERYONE
            | USE_EXTERNAL_EMOJIS
            | CONNECT
            | SPEAK
            | MUTE_MEMBERS
            | DEAFEN_MEMBERS
            | MOVE_MEMBERS
            | USE_VAD
            | CHANGE_NICKNAME
            | MANAGE_NICKNAMES
            | MANAGE_ROLES
            | MANAGE_THREADS
            | CREATE_PUBLIC_THREADS
            | CREATE_PRIVATE_THREADS
            | MODERATE_MEMBERS
    }

    /// Default permissions for the Owner role (id=4). All defined permission bits.
    pub fn owner_permissions() -> u64 {
        all_permissions()
    }

    use super::{OverwriteType, PermissionOverwrite, RoleDefinition};

    /// Calculate the effective permissions for a member in a specific channel.
    ///
    /// Follows Discord's 8-step permission calculation:
    /// 1. Start with @everyone base permissions
    /// 2. Apply role permissions (OR all role permissions together)
    /// 3. If ADMINISTRATOR, return ALL permissions
    /// 4. Apply @everyone channel overwrites
    /// 5. Apply role-specific channel overwrites (OR allow, then AND NOT deny)
    /// 6. Apply member-specific channel overwrites
    /// 7. If timed out, strip write permissions
    /// 8. If no `VIEW_CHANNEL`, strip all channel-specific permissions
    pub fn calculate_permissions(
        member_role_ids: &[u32],
        all_roles: &[RoleDefinition],
        channel_overwrites: &[PermissionOverwrite],
        member_pseudonym: &str,
        timeout_until: Option<u64>,
    ) -> u64 {
        // Step 1: Find @everyone role permissions
        let everyone_perms = all_roles
            .iter()
            .find(|r| r.id == super::ROLE_EVERYONE_ID)
            .map_or(0, |r| r.permissions);

        // Step 2: OR together all role permissions
        let mut base_permissions = everyone_perms;
        for role_id in member_role_ids {
            if *role_id == super::ROLE_EVERYONE_ID {
                continue; // already included
            }
            if let Some(role) = all_roles.iter().find(|r| r.id == *role_id) {
                base_permissions |= role.permissions;
            }
        }

        // Step 3: ADMINISTRATOR bypass
        if is_administrator(base_permissions) {
            return all_permissions();
        }

        // Steps 4-6: Apply channel overwrites
        let mut permissions = base_permissions;

        if !channel_overwrites.is_empty() {
            // Step 4: @everyone channel overwrite
            for ow in channel_overwrites {
                if ow.target_type == OverwriteType::Role
                    && ow.target_id == super::ROLE_EVERYONE_ID.to_string()
                {
                    permissions &= !ow.deny;
                    permissions |= ow.allow;
                }
            }

            // Step 5: Role-specific channel overwrites (accumulate then apply)
            let mut role_allow: u64 = 0;
            let mut role_deny: u64 = 0;
            for ow in channel_overwrites {
                if ow.target_type == OverwriteType::Role {
                    if let Ok(role_id) = ow.target_id.parse::<u32>() {
                        if role_id != super::ROLE_EVERYONE_ID
                            && member_role_ids.contains(&role_id)
                        {
                            role_allow |= ow.allow;
                            role_deny |= ow.deny;
                        }
                    }
                }
            }
            permissions &= !role_deny;
            permissions |= role_allow;

            // Step 6: Member-specific channel overwrite
            for ow in channel_overwrites {
                if ow.target_type == OverwriteType::Member && ow.target_id == member_pseudonym {
                    permissions &= !ow.deny;
                    permissions |= ow.allow;
                }
            }
        }

        // Step 7: If timed out, strip write/voice permissions
        if let Some(until) = timeout_until {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now < until {
                permissions &= !(SEND_MESSAGES
                    | ADD_REACTIONS
                    | SPEAK
                    | STREAM
                    | CREATE_INSTANT_INVITE);
            }
        }

        // Step 8: If no VIEW_CHANNEL, deny everything except non-channel perms
        if permissions & VIEW_CHANNEL == 0 {
            permissions = 0;
        }

        permissions
    }
}
