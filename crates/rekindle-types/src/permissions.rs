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

/// An individual permission in the role editor.
pub struct CatalogueEntry {
    pub key: &'static str,
    pub label: &'static str,
    pub bit: u64,
}

/// A role-editor permission group.
pub struct CatalogueGroup {
    pub name: &'static str,
    pub permissions: &'static [CatalogueEntry],
}

macro_rules! permission_entries {
    ($($bit:ident => $label:literal,)*) => {
        &[$(CatalogueEntry { key: stringify!($bit), label: $label, bit: $bit }),*]
    };
}

/// The Host's role-editor groups and entries, in display order.
pub const PERMISSION_CATALOGUE: &[CatalogueGroup] = &[
    CatalogueGroup {
        name: "General",
        permissions: permission_entries! {
            VIEW_CHANNELS => "View Channels",
            MANAGE_CHANNELS => "Manage Channels",
            MANAGE_ROLES => "Manage Roles",
            MANAGE_COMMUNITY => "Manage Community",
            CREATE_INVITES => "Create Invites",
            KICK_MEMBERS => "Kick Members",
            BAN_MEMBERS => "Ban Members",
            TIMEOUT_MEMBERS => "Moderate Members",
            VIEW_AUDIT_LOG => "View Audit Log",
            MANAGE_NICKNAMES => "Manage Nicknames",
            MANAGE_EXPRESSIONS => "Manage Expressions",
            VIEW_INSIGHTS => "View Insights",
            CREATE_EXPRESSIONS => "Create Expressions",
            ADMINISTRATOR => "Administrator",
        },
    },
    CatalogueGroup {
        name: "Text",
        permissions: permission_entries! {
            SEND_MESSAGES => "Send Messages",
            EMBED_LINKS => "Embed Links",
            ATTACH_FILES => "Attach Files",
            ADD_REACTIONS => "Add Reactions",
            MENTION_EVERYONE => "Mention Everyone",
            MANAGE_MESSAGES => "Manage Messages",
            READ_HISTORY => "Read History",
            PIN_MESSAGES => "Pin Messages",
            SEND_POLLS => "Send Polls",
            SEND_TTS => "Send TTS Messages",
            USE_EXTERNAL_EMOJIS => "Use External Emojis",
            USE_EXTERNAL_STICKERS => "Use External Stickers",
            SEND_VOICE_MESSAGES => "Send Voice Messages",
            BYPASS_SLOWMODE => "Bypass Slowmode",
        },
    },
    CatalogueGroup {
        name: "Voice",
        permissions: permission_entries! {
            CONNECT => "Connect",
            SPEAK => "Speak",
            MUTE_MEMBERS => "Mute Members",
            DEAFEN_MEMBERS => "Deafen Members",
            MOVE_MEMBERS => "Move Members",
            USE_VOICE_ACTIVITY => "Use Voice Activity",
            REQUEST_TO_SPEAK => "Request to Speak",
            STREAM => "Stream",
            PRIORITY_SPEAKER => "Priority Speaker",
            USE_SOUNDBOARD => "Use Soundboard",
            USE_EXTERNAL_SOUNDS => "Use External Sounds",
        },
    },
    CatalogueGroup {
        name: "Threads",
        permissions: permission_entries! {
            MANAGE_THREADS => "Manage Threads",
            CREATE_PUBLIC_THREADS => "Create Public Threads",
            CREATE_PRIVATE_THREADS => "Create Private Threads",
            SEND_IN_THREADS => "Send In Threads",
        },
    },
    CatalogueGroup {
        name: "Events",
        permissions: permission_entries! {
            MANAGE_EVENTS => "Manage Events",
            CREATE_EVENTS => "Create Events",
        },
    },
];

/// Role-editor labels indexed in ascending bit order for capability denials.
pub const PERMISSION_NAMES: &[(u64, &str)] = &permission_names();

const fn permission_names() -> [(u64, &'static str); DEFINED.count_ones() as usize] {
    let mut names = [(0, ""); DEFINED.count_ones() as usize];
    let mut group = 0;
    while group < PERMISSION_CATALOGUE.len() {
        let entries = PERMISSION_CATALOGUE[group].permissions;
        let mut index = 0;
        while index < entries.len() {
            let entry = &entries[index];
            let bit_index = (DEFINED & (entry.bit - 1)).count_ones() as usize;
            names[bit_index] = (entry.bit, entry.label);
            index += 1;
        }
        group += 1;
    }
    names
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
