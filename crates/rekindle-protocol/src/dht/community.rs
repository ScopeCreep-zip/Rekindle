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

pub const COMMUNITY_SUBKEY_COUNT: u32 = 6;

/// Community metadata stored in subkey 0.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityMetadata {
    pub name: String,
    pub description: Option<String>,
    pub icon_hash: Option<String>,
    pub created_at: u64,
    pub owner_key: String,
}

/// A channel entry stored in the channel list (subkey 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelEntry {
    pub id: String,
    pub name: String,
    pub channel_type: String, // "text" or "voice"
    pub sort_order: u16,
    pub latest_message_key: Option<String>,
}

/// A member entry stored in the member list (subkey 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberEntry {
    pub public_key: String,
    pub role_ids: Vec<u32>,
    pub joined_at: u64,
}

/// A role definition stored in the roles list (subkey 3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleDefinition {
    pub id: u32,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    pub sort_order: u16,
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

    // Initialize member list with owner
    let members = vec![MemberEntry {
        public_key: metadata.owner_key.clone(),
        role_ids: vec![0], // owner role
        joined_at: metadata.created_at,
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
    if members.iter().any(|m| m.public_key == member.public_key) {
        return Err(ProtocolError::DhtError("member already exists".into()));
    }
    members.push(member);
    let data = capnp_codec::community::encode_members(&members)?;
    dht.set_value(key, SUBKEY_MEMBERS, data).await
}

/// Remove a member from the community by public key.
pub async fn remove_member(
    dht: &DHTManager,
    key: &str,
    public_key: &str,
) -> Result<(), ProtocolError> {
    let mut members = read_members(dht, key).await?;
    let before = members.len();
    members.retain(|m| m.public_key != public_key);
    if members.len() == before {
        return Err(ProtocolError::PeerNotFound(format!(
            "member {public_key} not found"
        )));
    }
    let data = capnp_codec::community::encode_members(&members)?;
    dht.set_value(key, SUBKEY_MEMBERS, data).await
}

/// Assign a role to a member.
pub async fn assign_member_role(
    dht: &DHTManager,
    key: &str,
    public_key: &str,
    role_id: u32,
) -> Result<(), ProtocolError> {
    let mut members = read_members(dht, key).await?;
    let member = members
        .iter_mut()
        .find(|m| m.public_key == public_key)
        .ok_or_else(|| ProtocolError::PeerNotFound(format!("member {public_key} not found")))?;
    if !member.role_ids.contains(&role_id) {
        member.role_ids.push(role_id);
    }
    let data = capnp_codec::community::encode_members(&members)?;
    dht.set_value(key, SUBKEY_MEMBERS, data).await
}

/// Revoke a role from a member.
pub async fn revoke_member_role(
    dht: &DHTManager,
    key: &str,
    public_key: &str,
    role_id: u32,
) -> Result<(), ProtocolError> {
    let mut members = read_members(dht, key).await?;
    let member = members
        .iter_mut()
        .find(|m| m.public_key == public_key)
        .ok_or_else(|| ProtocolError::PeerNotFound(format!("member {public_key} not found")))?;
    member.role_ids.retain(|&id| id != role_id);
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

/// Permission bit flags for role-based access control.
pub mod permissions {
    pub const SEND_MESSAGES: u64 = 1 << 0;
    pub const MANAGE_CHANNELS: u64 = 1 << 1;
    pub const MANAGE_MEMBERS: u64 = 1 << 2;
    pub const MANAGE_ROLES: u64 = 1 << 3;
    pub const KICK_MEMBERS: u64 = 1 << 4;
    pub const BAN_MEMBERS: u64 = 1 << 5;
    pub const MANAGE_COMMUNITY: u64 = 1 << 6;
    pub const INVITE_MEMBERS: u64 = 1 << 7;
    pub const SPEAK_IN_VOICE: u64 = 1 << 8;

    /// Check if a permission bitmask includes a specific permission.
    pub fn has_permission(member_permissions: u64, required: u64) -> bool {
        member_permissions & required == required
    }

    /// Default permissions for the owner role.
    pub fn owner_permissions() -> u64 {
        u64::MAX // All permissions
    }

    /// Default permissions for regular members.
    pub fn member_permissions() -> u64 {
        SEND_MESSAGES | SPEAK_IN_VOICE
    }
}
