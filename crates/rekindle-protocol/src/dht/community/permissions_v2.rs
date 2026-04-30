//! V2 permission system using `bitflags` for type-safe permission manipulation.
//!
//! Provides the same Discord-aligned bit positions as the v1 `permissions` module
//! but with a proper `Permissions` newtype, a full 8-step `calculate_permissions_v2`
//! function, and default permission presets.

use super::types::RoleEntryV2;
use super::{OverwriteType, PermissionOverwrite, ROLE_EVERYONE_ID};
use bitflags::bitflags;

bitflags! {
    /// Type-safe permission bitmask for community role-based access control.
    ///
    /// Bit positions are Discord-aligned. The highest defined bit is 57
    /// (`VIEW_CREATOR_MONETIZATION_ANALYTICS`), well within the u64 range.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Permissions: u64 {
        // ── General ──
        const CREATE_INSTANT_INVITE     = 1 << 0;
        const KICK_MEMBERS              = 1 << 1;
        const BAN_MEMBERS               = 1 << 2;
        const ADMINISTRATOR             = 1 << 3;
        const MANAGE_CHANNELS           = 1 << 4;
        const MANAGE_COMMUNITY          = 1 << 5;

        // ── Text ──
        const ADD_REACTIONS             = 1 << 6;
        const VIEW_AUDIT_LOG            = 1 << 7;
        const PRIORITY_SPEAKER          = 1 << 8;
        const STREAM                    = 1 << 9;
        const VIEW_CHANNEL              = 1 << 10;
        const SEND_MESSAGES             = 1 << 11;
        // bit 12 reserved
        const MANAGE_MESSAGES           = 1 << 13;
        const EMBED_LINKS               = 1 << 14;
        const ATTACH_FILES              = 1 << 15;
        const READ_MESSAGE_HISTORY      = 1 << 16;
        const MENTION_EVERYONE          = 1 << 17;
        const USE_EXTERNAL_EMOJIS       = 1 << 18;
        // bit 19 reserved

        // ── Voice ──
        const CONNECT                   = 1 << 20;
        const SPEAK                     = 1 << 21;
        const MUTE_MEMBERS              = 1 << 22;
        const DEAFEN_MEMBERS            = 1 << 23;
        const MOVE_MEMBERS              = 1 << 24;
        const USE_VAD                   = 1 << 25;

        // ── Membership ──
        const CHANGE_NICKNAME           = 1 << 26;
        const MANAGE_NICKNAMES          = 1 << 27;
        const MANAGE_ROLES              = 1 << 28;
        // bits 29-32 reserved

        // ── Events ──
        const MANAGE_EVENTS             = 1 << 33;

        // ── Threads ──
        const MANAGE_THREADS            = 1 << 34;
        const CREATE_PUBLIC_THREADS     = 1 << 35;
        const CREATE_PRIVATE_THREADS    = 1 << 36;
        // bits 37-39 reserved

        // ── Moderation ──
        const MODERATE_MEMBERS          = 1 << 40;
        // bits 41-42 reserved

        // ── Advanced ──
        const USE_APPLICATION_COMMANDS  = 1 << 43;
        const REQUEST_TO_SPEAK          = 1 << 44;
        const MANAGE_GUILD_EXPRESSIONS  = 1 << 45;
        const MANAGE_WEBHOOKS           = 1 << 46;
        // bits 47-48 reserved
        const CREATE_GUILD_EXPRESSIONS  = 1 << 49;
        // bits 50-52 reserved
        const SEND_VOICE_MESSAGES       = 1 << 53;
        // bits 54-55 reserved
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
        let bits = u64::deserialize(deserializer)?;
        Ok(Self::from_bits_truncate(bits))
    }
}

impl Permissions {
    /// Check if this permission set includes the required permission.
    /// Returns `true` immediately if ADMINISTRATOR is set.
    pub fn has(self, required: Self) -> bool {
        if self.contains(Self::ADMINISTRATOR) {
            return true;
        }
        self.contains(required)
    }

    /// Check if this is an administrator permission set.
    pub fn is_administrator(self) -> bool {
        self.contains(Self::ADMINISTRATOR)
    }
}

/// Backward-compatible wrapper: accepts raw u64, returns bool.
pub fn has_permission_v2(member_permissions: u64, required: u64) -> bool {
    let perms = Permissions::from_bits_truncate(member_permissions);
    let req = Permissions::from_bits_truncate(required);
    perms.has(req)
}

// ── Default permission presets ──

/// Default permissions for the @everyone role (id=0).
pub fn everyone_default() -> Permissions {
    Permissions::VIEW_CHANNEL
        | Permissions::READ_MESSAGE_HISTORY
        | Permissions::CONNECT
        | Permissions::SEND_MESSAGES
        | Permissions::SPEAK
        | Permissions::ADD_REACTIONS
        | Permissions::EMBED_LINKS
        | Permissions::ATTACH_FILES
        | Permissions::USE_EXTERNAL_EMOJIS
        | Permissions::USE_VAD
        | Permissions::CHANGE_NICKNAME
        | Permissions::SEND_VOICE_MESSAGES
        | Permissions::SEND_POLLS
}

/// Default permissions for the Member role (id=1).
pub fn member_default() -> Permissions {
    everyone_default()
        | Permissions::CREATE_INSTANT_INVITE
        | Permissions::CREATE_PUBLIC_THREADS
        | Permissions::USE_APPLICATION_COMMANDS
}

/// Default permissions for the Moderator role (id=2).
pub fn moderator_default() -> Permissions {
    member_default()
        | Permissions::KICK_MEMBERS
        | Permissions::MANAGE_MESSAGES
        | Permissions::MUTE_MEMBERS
        | Permissions::DEAFEN_MEMBERS
        | Permissions::MODERATE_MEMBERS
        | Permissions::MANAGE_EVENTS
        | Permissions::MANAGE_THREADS
}

/// Default permissions for the Admin role (id=3).
///
/// Includes `ADMINISTRATOR` so that admins are eligible for coordinator
/// election — required for the "community survives owner leaving" design.
pub fn admin_default() -> Permissions {
    moderator_default()
        | Permissions::ADMINISTRATOR
        | Permissions::MANAGE_CHANNELS
        | Permissions::MANAGE_ROLES
        | Permissions::BAN_MEMBERS
        | Permissions::VIEW_AUDIT_LOG
        | Permissions::MANAGE_NICKNAMES
        | Permissions::MANAGE_COMMUNITY
        | Permissions::MANAGE_GUILD_EXPRESSIONS
        | Permissions::MANAGE_WEBHOOKS
}

/// Default permissions for the Owner role (id=4).
pub fn owner_default() -> Permissions {
    Permissions::all()
}

/// Calculate the effective permissions for a member in a specific channel.
///
/// Implements the full 8-step Discord permission algorithm:
/// 1. Start with @everyone base permissions
/// 2. Apply role permissions (OR all role permissions together)
/// 3. If ADMINISTRATOR, return ALL permissions
/// 4. Apply @everyone channel overwrites
/// 5. Apply role-specific channel overwrites (OR allow, then AND NOT deny)
/// 6. Apply member-specific channel overwrites
/// 7. If timed out, strip write/voice permissions
/// 8. If no VIEW_CHANNEL, strip all
pub fn calculate_permissions_v2(
    member_role_ids: &[u32],
    all_roles: &[RoleEntryV2],
    channel_overwrites: &[PermissionOverwrite],
    member_pseudonym: &str,
    is_owner: bool,
    timeout_until: Option<u64>,
) -> Permissions {
    // Owner always has all permissions — this is the Discord "server owner" bypass.
    if is_owner {
        return Permissions::all();
    }

    // Step 1: Find @everyone role permissions
    let everyone_perms = all_roles
        .iter()
        .find(|r| r.id == ROLE_EVERYONE_ID)
        .map_or(Permissions::empty(), |r| {
            Permissions::from_bits_truncate(r.permissions)
        });

    // Step 2: OR together all assigned role permissions
    let mut base = everyone_perms;
    for role_id in member_role_ids {
        if *role_id == ROLE_EVERYONE_ID {
            continue;
        }
        if let Some(role) = all_roles.iter().find(|r| r.id == *role_id) {
            base |= Permissions::from_bits_truncate(role.permissions);
        }
    }

    // Step 3: ADMINISTRATOR bypass
    if base.is_administrator() {
        return Permissions::all();
    }

    let mut perms = base;

    // Steps 4-6: Channel overwrites
    if !channel_overwrites.is_empty() {
        // Step 4: @everyone channel overwrite
        let everyone_id_str = ROLE_EVERYONE_ID.to_string();
        for ow in channel_overwrites {
            if ow.target_type == OverwriteType::Role && ow.target_id == everyone_id_str {
                perms &= !Permissions::from_bits_truncate(ow.deny);
                perms |= Permissions::from_bits_truncate(ow.allow);
            }
        }

        // Step 5: Role-specific channel overwrites (accumulate then apply)
        let mut role_allow = Permissions::empty();
        let mut role_deny = Permissions::empty();
        for ow in channel_overwrites {
            if ow.target_type == OverwriteType::Role {
                if let Ok(role_id) = ow.target_id.parse::<u32>() {
                    if role_id != ROLE_EVERYONE_ID && member_role_ids.contains(&role_id) {
                        role_allow |= Permissions::from_bits_truncate(ow.allow);
                        role_deny |= Permissions::from_bits_truncate(ow.deny);
                    }
                }
            }
        }
        perms &= !role_deny;
        perms |= role_allow;

        // Step 6: Member-specific channel overwrite
        for ow in channel_overwrites {
            if ow.target_type == OverwriteType::Member && ow.target_id == member_pseudonym {
                perms &= !Permissions::from_bits_truncate(ow.deny);
                perms |= Permissions::from_bits_truncate(ow.allow);
            }
        }
    }

    // Step 7: If timed out, strip write/voice permissions
    if let Some(until) = timeout_until {
        let now = rekindle_utils::timestamp_secs();
        if now < until {
            perms &= !(Permissions::SEND_MESSAGES
                | Permissions::ADD_REACTIONS
                | Permissions::SPEAK
                | Permissions::STREAM
                | Permissions::CREATE_INSTANT_INVITE
                | Permissions::SEND_VOICE_MESSAGES
                | Permissions::SEND_POLLS);
        }
    }

    // Step 8: If no VIEW_CHANNEL, deny everything
    if !perms.contains(Permissions::VIEW_CHANNEL) {
        return Permissions::empty();
    }

    perms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_serde_roundtrip() {
        let p = Permissions::SEND_MESSAGES | Permissions::VIEW_CHANNEL;
        let json = serde_json::to_string(&p).unwrap();
        let back: Permissions = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn administrator_bypasses_all() {
        let p = Permissions::ADMINISTRATOR;
        assert!(p.has(Permissions::MANAGE_CHANNELS));
        assert!(p.has(Permissions::BAN_MEMBERS));
        assert!(p.has(Permissions::MODERATE_MEMBERS));
    }

    #[test]
    fn has_without_admin_checks_exact() {
        let p = Permissions::SEND_MESSAGES;
        assert!(p.has(Permissions::SEND_MESSAGES));
        assert!(!p.has(Permissions::MANAGE_CHANNELS));
    }

    #[test]
    fn calculate_basic_member() {
        let roles = vec![
            RoleEntryV2 {
                id: 0,
                name: "@everyone".into(),
                color: 0,
                permissions: everyone_default().bits(),
                position: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
            },
            RoleEntryV2 {
                id: 1,
                name: "Member".into(),
                color: 0,
                permissions: member_default().bits(),
                position: 1,
                hoist: false,
                mentionable: false,
                self_assignable: false,
            },
        ];

        let perms = calculate_permissions_v2(&[0, 1], &roles, &[], "alice", false, None);
        assert!(perms.contains(Permissions::VIEW_CHANNEL));
        assert!(perms.contains(Permissions::SEND_MESSAGES));
        assert!(perms.contains(Permissions::CREATE_INSTANT_INVITE));
        assert!(!perms.contains(Permissions::MANAGE_CHANNELS));
    }

    #[test]
    fn calculate_admin_gets_all() {
        let roles = vec![RoleEntryV2 {
            id: 3,
            name: "Admin".into(),
            color: 0,
            permissions: Permissions::ADMINISTRATOR.bits(),
            position: 3,
            hoist: false,
            mentionable: false,
            self_assignable: false,
        }];

        let perms = calculate_permissions_v2(&[3], &roles, &[], "admin_user", false, None);
        assert_eq!(perms, Permissions::all());
    }

    #[test]
    fn calculate_owner_bypass() {
        let perms = calculate_permissions_v2(&[], &[], &[], "owner", true, None);
        assert_eq!(perms, Permissions::all());
    }

    #[test]
    fn channel_overwrite_deny() {
        let roles = vec![RoleEntryV2 {
            id: 0,
            name: "@everyone".into(),
            color: 0,
            permissions: (Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES).bits(),
            position: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
        }];

        let overwrites = vec![PermissionOverwrite {
            target_type: OverwriteType::Role,
            target_id: "0".into(),
            allow: 0,
            deny: Permissions::SEND_MESSAGES.bits(),
        }];

        let perms = calculate_permissions_v2(&[0], &roles, &overwrites, "user", false, None);
        assert!(perms.contains(Permissions::VIEW_CHANNEL));
        assert!(!perms.contains(Permissions::SEND_MESSAGES));
    }

    #[test]
    fn member_overwrite_takes_priority() {
        let roles = vec![RoleEntryV2 {
            id: 0,
            name: "@everyone".into(),
            color: 0,
            permissions: Permissions::VIEW_CHANNEL.bits(),
            position: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
        }];

        let overwrites = vec![
            PermissionOverwrite {
                target_type: OverwriteType::Role,
                target_id: "0".into(),
                allow: 0,
                deny: Permissions::SEND_MESSAGES.bits(),
            },
            PermissionOverwrite {
                target_type: OverwriteType::Member,
                target_id: "special_user".into(),
                allow: Permissions::SEND_MESSAGES.bits(),
                deny: 0,
            },
        ];

        let perms =
            calculate_permissions_v2(&[0], &roles, &overwrites, "special_user", false, None);
        assert!(perms.contains(Permissions::SEND_MESSAGES));
    }

    #[test]
    fn no_view_channel_strips_all() {
        let roles = vec![RoleEntryV2 {
            id: 0,
            name: "@everyone".into(),
            color: 0,
            permissions: Permissions::SEND_MESSAGES.bits(), // no VIEW_CHANNEL
            position: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
        }];

        let perms = calculate_permissions_v2(&[0], &roles, &[], "user", false, None);
        assert_eq!(perms, Permissions::empty());
    }

    #[test]
    fn timeout_strips_write_perms() {
        let roles = vec![RoleEntryV2 {
            id: 0,
            name: "@everyone".into(),
            color: 0,
            permissions: (Permissions::VIEW_CHANNEL
                | Permissions::SEND_MESSAGES
                | Permissions::SPEAK)
                .bits(),
            position: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
        }];

        // Set timeout far in the future
        let timeout = rekindle_utils::timestamp_secs() + 3600;
        let perms = calculate_permissions_v2(&[0], &roles, &[], "user", false, Some(timeout));
        assert!(perms.contains(Permissions::VIEW_CHANNEL));
        assert!(!perms.contains(Permissions::SEND_MESSAGES));
        assert!(!perms.contains(Permissions::SPEAK));
    }

    #[test]
    fn has_permission_v2_compat() {
        // Test backward-compatible wrapper
        let perms = (Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES).bits();
        assert!(has_permission_v2(perms, Permissions::SEND_MESSAGES.bits()));
        assert!(!has_permission_v2(
            perms,
            Permissions::MANAGE_CHANNELS.bits()
        ));
        // ADMINISTRATOR bypass
        assert!(has_permission_v2(
            Permissions::ADMINISTRATOR.bits(),
            Permissions::MANAGE_CHANNELS.bits()
        ));
    }

    #[test]
    fn high_bit_permissions() {
        // Verify bits above 32 work correctly
        let p = Permissions::MODERATE_MEMBERS
            | Permissions::VIEW_CREATOR_MONETIZATION_ANALYTICS
            | Permissions::SEND_POLLS;
        assert!(p.contains(Permissions::MODERATE_MEMBERS));
        assert!(p.contains(Permissions::SEND_POLLS));
        assert!(p.contains(Permissions::VIEW_CREATOR_MONETIZATION_ANALYTICS));
        assert!(!p.contains(Permissions::SEND_MESSAGES));
    }

    #[test]
    fn preset_hierarchy() {
        // Each preset should be a superset of the one below it
        let everyone = everyone_default();
        let member = member_default();
        let moderator = moderator_default();
        let admin = admin_default();
        let owner = owner_default();

        assert!(member.contains(everyone));
        assert!(moderator.contains(member));
        assert!(admin.contains(moderator));
        assert!(owner.contains(admin));
    }
}
