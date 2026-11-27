//! v2.0 Permission bitmask constants.
//!
//! 64-bit permission bitmask matching the architecture doc §9.1.
//! Used by `rekindle-governance` for permission computation and by
//! the reader-validates model for gossip message validation.

// Keep the declared-bit mask derived from the same declarations as the constants.
macro_rules! permission_bits {
    ($($(#[$attr:meta])* $name:ident = $bit:literal;)*) => {
        $($(#[$attr])* pub const $name: u64 = 1 << $bit;)*

        /// All defined permission bits, excluding reserved positions.
        pub const DEFINED: u64 = 0 $(| $name)*;
    };
}

permission_bits! {
    /// General permissions (bits 0-15)
    VIEW_CHANNELS = 0;
    MANAGE_CHANNELS = 1;
    MANAGE_ROLES = 2;
    MANAGE_COMMUNITY = 3;
    CREATE_INVITES = 4;
    KICK_MEMBERS = 5;
    BAN_MEMBERS = 6;
    TIMEOUT_MEMBERS = 7;
    MANAGE_NICKNAMES = 8;
    MANAGE_EXPRESSIONS = 9;
    VIEW_AUDIT_LOG = 10;
    VIEW_INSIGHTS = 11;
    CREATE_EXPRESSIONS = 12;

    /// Text permissions (bits 16-31)
    SEND_MESSAGES = 16;
    EMBED_LINKS = 17;
    ATTACH_FILES = 18;
    ADD_REACTIONS = 19;
    MENTION_EVERYONE = 20;
    MANAGE_MESSAGES = 21;
    READ_HISTORY = 22;
    SEND_TTS = 23;
    USE_EXTERNAL_EMOJIS = 24;
    USE_EXTERNAL_STICKERS = 25;
    PIN_MESSAGES = 26;
    SEND_VOICE_MESSAGES = 27;
    SEND_POLLS = 28;
    BYPASS_SLOWMODE = 29;

    /// Voice permissions (bits 32-43)
    CONNECT = 32;
    SPEAK = 33;
    MUTE_MEMBERS = 34;
    DEAFEN_MEMBERS = 35;
    MOVE_MEMBERS = 36;
    USE_VOICE_ACTIVITY = 37;
    PRIORITY_SPEAKER = 38;
    USE_SOUNDBOARD = 39;
    USE_EXTERNAL_SOUNDS = 40;
    REQUEST_TO_SPEAK = 41;
    STREAM = 42;

    /// Thread permissions (bits 44-47)
    MANAGE_THREADS = 44;
    CREATE_PUBLIC_THREADS = 45;
    CREATE_PRIVATE_THREADS = 46;
    SEND_IN_THREADS = 47;

    /// Event permissions (bits 48-49)
    MANAGE_EVENTS = 48;
    CREATE_EVENTS = 49;

    /// Admin permission (bit 50) — bypasses all checks.
    ADMINISTRATOR = 50;
}

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

/// The permissions any one of which lets a member rotate the community
/// key ([`Permissions::may_rotate_mek`]).
pub const MEK_ROTATION: u64 = KICK_MEMBERS | BAN_MEMBERS | MANAGE_COMMUNITY;

/// All permission positions (bits 0–50), including reserved positions.
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

    /// May this member rotate the community key — be elected rotator and
    /// write a `MEKGenerationBump` (plan D20)? Any one of
    /// [`MEK_ROTATION`] suffices: rotation follows a removal, and the
    /// members who can remove someone are the ones trusted to re-key.
    pub fn may_rotate_mek(self) -> bool {
        self.is_administrator() || self.0 & MEK_ROTATION != 0
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
#[path = "permissions_tests.rs"]
mod tests;
