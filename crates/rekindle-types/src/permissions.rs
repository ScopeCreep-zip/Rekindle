//! v2.0 Permission bitmask constants.
//!
//! 64-bit permission bitmask matching the architecture doc §9.1.
//! Used by `rekindle-governance` for permission computation and by
//! the reader-validates model for gossip message validation.

/// General permissions (bits 0-15)
pub const VIEW_CHANNELS: u64 = 1 << 0;
pub const MANAGE_CHANNELS: u64 = 1 << 1;
pub const MANAGE_ROLES: u64 = 1 << 2;
pub const MANAGE_COMMUNITY: u64 = 1 << 3;
pub const CREATE_INVITES: u64 = 1 << 4;
pub const KICK_MEMBERS: u64 = 1 << 5;
pub const BAN_MEMBERS: u64 = 1 << 6;
pub const TIMEOUT_MEMBERS: u64 = 1 << 7;
pub const MANAGE_NICKNAMES: u64 = 1 << 8;
pub const MANAGE_EXPRESSIONS: u64 = 1 << 9;
pub const VIEW_AUDIT_LOG: u64 = 1 << 10;
pub const VIEW_INSIGHTS: u64 = 1 << 11;

/// Text permissions (bits 16-31)
pub const SEND_MESSAGES: u64 = 1 << 16;
pub const EMBED_LINKS: u64 = 1 << 17;
pub const ATTACH_FILES: u64 = 1 << 18;
pub const ADD_REACTIONS: u64 = 1 << 19;
pub const MENTION_EVERYONE: u64 = 1 << 20;
pub const MANAGE_MESSAGES: u64 = 1 << 21;
pub const READ_HISTORY: u64 = 1 << 22;
pub const SEND_TTS: u64 = 1 << 23;
pub const USE_EXTERNAL_EMOJIS: u64 = 1 << 24;
pub const USE_EXTERNAL_STICKERS: u64 = 1 << 25;
pub const PIN_MESSAGES: u64 = 1 << 26;
pub const SEND_VOICE_MESSAGES: u64 = 1 << 27;
pub const SEND_POLLS: u64 = 1 << 28;
pub const BYPASS_SLOWMODE: u64 = 1 << 29;

/// Voice permissions (bits 32-43)
pub const CONNECT: u64 = 1 << 32;
pub const SPEAK: u64 = 1 << 33;
pub const MUTE_MEMBERS: u64 = 1 << 34;
pub const DEAFEN_MEMBERS: u64 = 1 << 35;
pub const MOVE_MEMBERS: u64 = 1 << 36;
pub const USE_VOICE_ACTIVITY: u64 = 1 << 37;
pub const PRIORITY_SPEAKER: u64 = 1 << 38;
pub const USE_SOUNDBOARD: u64 = 1 << 39;
pub const USE_EXTERNAL_SOUNDS: u64 = 1 << 40;
pub const REQUEST_TO_SPEAK: u64 = 1 << 41;
pub const STREAM: u64 = 1 << 42;

/// Thread permissions (bits 44-47)
pub const MANAGE_THREADS: u64 = 1 << 44;
pub const CREATE_PUBLIC_THREADS: u64 = 1 << 45;
pub const CREATE_PRIVATE_THREADS: u64 = 1 << 46;
pub const SEND_IN_THREADS: u64 = 1 << 47;

/// Event permissions (bits 48-49)
pub const MANAGE_EVENTS: u64 = 1 << 48;
pub const CREATE_EVENTS: u64 = 1 << 49;

/// Admin permission (bit 50) — bypasses all checks.
pub const ADMINISTRATOR: u64 = 1 << 50;

/// Default permissions for the @everyone role in new communities.
pub const DEFAULT_EVERYONE: u64 = VIEW_CHANNELS
    | SEND_MESSAGES
    | READ_HISTORY
    | ADD_REACTIONS
    | EMBED_LINKS
    | ATTACH_FILES
    | USE_VOICE_ACTIVITY
    | CONNECT
    | SPEAK
    | CREATE_INVITES
    | CREATE_PUBLIC_THREADS
    | SEND_IN_THREADS
    | CREATE_EVENTS;

/// All permissions set (bits 0-50).
pub const ALL: u64 = (1 << 51) - 1;

/// Convenience struct wrapping a u64 bitmask with helper methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions(pub u64);

impl Permissions {
    /// Check if this permission set includes the required permission.
    /// Returns true immediately if ADMINISTRATOR is set.
    pub fn has(self, required: u64) -> bool {
        if self.0 & ADMINISTRATOR != 0 {
            return true;
        }
        self.0 & required == required
    }

    /// Check if ADMINISTRATOR bit is set.
    pub fn is_administrator(self) -> bool {
        self.0 & ADMINISTRATOR != 0
    }
}

impl From<u64> for Permissions {
    fn from(bits: u64) -> Self {
        Self(bits)
    }
}

impl From<Permissions> for u64 {
    fn from(p: Permissions) -> Self {
        p.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn administrator_bypasses_all() {
        let perms = Permissions(ADMINISTRATOR);
        assert!(perms.has(SEND_MESSAGES));
        assert!(perms.has(BAN_MEMBERS));
        assert!(perms.has(MANAGE_CHANNELS));
    }

    #[test]
    fn specific_permission_check() {
        let perms = Permissions(SEND_MESSAGES | ADD_REACTIONS);
        assert!(perms.has(SEND_MESSAGES));
        assert!(perms.has(ADD_REACTIONS));
        assert!(!perms.has(BAN_MEMBERS));
    }

    #[test]
    fn default_everyone_includes_basics() {
        let perms = Permissions(DEFAULT_EVERYONE);
        assert!(perms.has(VIEW_CHANNELS));
        assert!(perms.has(SEND_MESSAGES));
        assert!(perms.has(READ_HISTORY));
        assert!(perms.has(CONNECT));
        assert!(perms.has(SPEAK));
        assert!(!perms.has(BAN_MEMBERS));
        assert!(!perms.has(ADMINISTRATOR));
    }

    #[test]
    fn bits_dont_overlap() {
        // Verify no two constants share a bit
        let all_consts = [
            VIEW_CHANNELS, MANAGE_CHANNELS, MANAGE_ROLES, MANAGE_COMMUNITY,
            CREATE_INVITES, KICK_MEMBERS, BAN_MEMBERS, TIMEOUT_MEMBERS,
            MANAGE_NICKNAMES, MANAGE_EXPRESSIONS, VIEW_AUDIT_LOG, VIEW_INSIGHTS,
            SEND_MESSAGES, EMBED_LINKS, ATTACH_FILES, ADD_REACTIONS,
            MENTION_EVERYONE, MANAGE_MESSAGES, READ_HISTORY, SEND_TTS,
            USE_EXTERNAL_EMOJIS, USE_EXTERNAL_STICKERS, PIN_MESSAGES,
            SEND_VOICE_MESSAGES, SEND_POLLS, BYPASS_SLOWMODE,
            CONNECT, SPEAK, MUTE_MEMBERS, DEAFEN_MEMBERS, MOVE_MEMBERS,
            USE_VOICE_ACTIVITY, PRIORITY_SPEAKER, USE_SOUNDBOARD,
            USE_EXTERNAL_SOUNDS, REQUEST_TO_SPEAK, STREAM,
            MANAGE_THREADS, CREATE_PUBLIC_THREADS, CREATE_PRIVATE_THREADS, SEND_IN_THREADS,
            MANAGE_EVENTS, CREATE_EVENTS,
            ADMINISTRATOR,
        ];
        for (i, a) in all_consts.iter().enumerate() {
            for b in &all_consts[i + 1..] {
                assert_eq!(a & b, 0, "Permission bits overlap: {a:#x} and {b:#x}");
            }
        }
    }
}
