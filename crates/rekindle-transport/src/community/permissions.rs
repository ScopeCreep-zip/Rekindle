//! Permission bitmask constants, preset roles, and the full 8-step
//! Discord-aligned permission computation algorithm.

use serde::{Deserialize, Serialize};

use crate::payload::dht_types::RoleEntry;

/// The @everyone role always has ID 0.
pub const ROLE_EVERYONE_ID: u32 = 0;

/// Permission overwrite for a channel, targeting either a role or a member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionOverwrite {
    pub target_type: OverwriteType,
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

bitflags::bitflags! {
    /// Type-safe permission bitmask. Discord-aligned bit positions.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Permissions: u64 {
        const CREATE_INSTANT_INVITE     = 1 << 0;
        const KICK_MEMBERS              = 1 << 1;
        const BAN_MEMBERS               = 1 << 2;
        const ADMINISTRATOR             = 1 << 3;
        const MANAGE_CHANNELS           = 1 << 4;
        const MANAGE_COMMUNITY          = 1 << 5;
        const ADD_REACTIONS             = 1 << 6;
        const VIEW_AUDIT_LOG            = 1 << 7;
        const PRIORITY_SPEAKER          = 1 << 8;
        const STREAM                    = 1 << 9;
        const VIEW_CHANNEL              = 1 << 10;
        const SEND_MESSAGES             = 1 << 11;
        const MANAGE_MESSAGES           = 1 << 13;
        const EMBED_LINKS               = 1 << 14;
        const ATTACH_FILES              = 1 << 15;
        const READ_MESSAGE_HISTORY      = 1 << 16;
        const MENTION_EVERYONE          = 1 << 17;
        const USE_EXTERNAL_EMOJIS       = 1 << 18;
        const CONNECT                   = 1 << 20;
        const SPEAK                     = 1 << 21;
        const MUTE_MEMBERS              = 1 << 22;
        const DEAFEN_MEMBERS            = 1 << 23;
        const MOVE_MEMBERS              = 1 << 24;
        const USE_VAD                   = 1 << 25;
        const CHANGE_NICKNAME           = 1 << 26;
        const MANAGE_NICKNAMES          = 1 << 27;
        const MANAGE_ROLES              = 1 << 28;
        const MANAGE_EVENTS             = 1 << 33;
        const MANAGE_THREADS            = 1 << 34;
        const CREATE_PUBLIC_THREADS     = 1 << 35;
        const CREATE_PRIVATE_THREADS    = 1 << 36;
        const MODERATE_MEMBERS          = 1 << 40;
        const USE_APPLICATION_COMMANDS  = 1 << 43;
        const REQUEST_TO_SPEAK          = 1 << 44;
        const MANAGE_GUILD_EXPRESSIONS  = 1 << 45;
        const MANAGE_WEBHOOKS           = 1 << 46;
        const CREATE_GUILD_EXPRESSIONS  = 1 << 49;
        const SEND_VOICE_MESSAGES       = 1 << 53;
        const SEND_POLLS                = 1 << 56;
        const VIEW_CREATOR_MONETIZATION_ANALYTICS = 1 << 57;
    }
}

impl serde::Serialize for Permissions {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.bits().serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Permissions {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_bits_truncate(u64::deserialize(deserializer)?))
    }
}

impl Permissions {
    pub fn has(self, required: Self) -> bool {
        self.contains(Self::ADMINISTRATOR) || self.contains(required)
    }
}

/// Check permission from raw u64 values (backward-compatible wrapper).
pub fn has_permission(member_permissions: u64, required: u64) -> bool {
    Permissions::from_bits_truncate(member_permissions)
        .has(Permissions::from_bits_truncate(required))
}

// ── Default permission presets ───────────────────────────────────────

pub fn everyone_default() -> Permissions {
    Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY | Permissions::CONNECT
        | Permissions::SEND_MESSAGES | Permissions::SPEAK | Permissions::ADD_REACTIONS
        | Permissions::EMBED_LINKS | Permissions::ATTACH_FILES | Permissions::USE_EXTERNAL_EMOJIS
        | Permissions::USE_VAD | Permissions::CHANGE_NICKNAME | Permissions::SEND_VOICE_MESSAGES
        | Permissions::SEND_POLLS
}

pub fn member_default() -> Permissions {
    everyone_default() | Permissions::CREATE_INSTANT_INVITE
        | Permissions::CREATE_PUBLIC_THREADS | Permissions::USE_APPLICATION_COMMANDS
}

pub fn moderator_default() -> Permissions {
    member_default() | Permissions::KICK_MEMBERS | Permissions::MANAGE_MESSAGES
        | Permissions::MUTE_MEMBERS | Permissions::DEAFEN_MEMBERS
        | Permissions::MODERATE_MEMBERS | Permissions::MANAGE_EVENTS | Permissions::MANAGE_THREADS
}

pub fn admin_default() -> Permissions {
    moderator_default() | Permissions::ADMINISTRATOR | Permissions::MANAGE_CHANNELS
        | Permissions::MANAGE_ROLES | Permissions::BAN_MEMBERS | Permissions::VIEW_AUDIT_LOG
        | Permissions::MANAGE_NICKNAMES | Permissions::MANAGE_COMMUNITY
        | Permissions::MANAGE_GUILD_EXPRESSIONS | Permissions::MANAGE_WEBHOOKS
}

pub fn owner_default() -> Permissions { Permissions::all() }

/// Full 8-step Discord permission algorithm.
pub fn calculate_permissions(
    member_role_ids: &[u32],
    all_roles: &[RoleEntry],
    channel_overwrites: &[PermissionOverwrite],
    member_pseudonym: &str,
    is_owner: bool,
    timeout_until: Option<u64>,
) -> Permissions {
    if is_owner { return Permissions::all(); }

    // Step 1: @everyone base
    let everyone_perms = all_roles.iter()
        .find(|r| r.id == ROLE_EVERYONE_ID)
        .map_or(Permissions::empty(), |r| Permissions::from_bits_truncate(r.permissions));

    // Step 2: OR all assigned roles
    let mut base = everyone_perms;
    for rid in member_role_ids {
        if *rid != ROLE_EVERYONE_ID {
            if let Some(role) = all_roles.iter().find(|r| r.id == *rid) {
                base |= Permissions::from_bits_truncate(role.permissions);
            }
        }
    }

    // Step 3: ADMINISTRATOR bypass
    if base.contains(Permissions::ADMINISTRATOR) { return Permissions::all(); }

    let mut perms = base;

    if !channel_overwrites.is_empty() {
        let eid = ROLE_EVERYONE_ID.to_string();
        // Step 4: @everyone channel overwrite
        for ow in channel_overwrites {
            if ow.target_type == OverwriteType::Role && ow.target_id == eid {
                perms &= !Permissions::from_bits_truncate(ow.deny);
                perms |= Permissions::from_bits_truncate(ow.allow);
            }
        }
        // Step 5: Role-specific overwrites
        let (mut ra, mut rd) = (Permissions::empty(), Permissions::empty());
        for ow in channel_overwrites {
            if ow.target_type == OverwriteType::Role {
                if let Ok(rid) = ow.target_id.parse::<u32>() {
                    if rid != ROLE_EVERYONE_ID && member_role_ids.contains(&rid) {
                        ra |= Permissions::from_bits_truncate(ow.allow);
                        rd |= Permissions::from_bits_truncate(ow.deny);
                    }
                }
            }
        }
        perms &= !rd;
        perms |= ra;
        // Step 6: Member-specific overwrite
        for ow in channel_overwrites {
            if ow.target_type == OverwriteType::Member && ow.target_id == member_pseudonym {
                perms &= !Permissions::from_bits_truncate(ow.deny);
                perms |= Permissions::from_bits_truncate(ow.allow);
            }
        }
    }

    // Step 7: Timeout strips write/voice
    if let Some(until) = timeout_until {
        if rekindle_utils::timestamp_secs() < until {
            perms &= !(Permissions::SEND_MESSAGES | Permissions::ADD_REACTIONS
                | Permissions::SPEAK | Permissions::STREAM
                | Permissions::CREATE_INSTANT_INVITE | Permissions::SEND_VOICE_MESSAGES
                | Permissions::SEND_POLLS);
        }
    }

    // Step 8: No VIEW_CHANNEL → deny all
    if !perms.contains(Permissions::VIEW_CHANNEL) { return Permissions::empty(); }

    perms
}
