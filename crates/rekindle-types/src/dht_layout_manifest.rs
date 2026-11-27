pub const METADATA: u32 = 0;
pub const CHANNELS: u32 = 1;
pub const CATEGORIES: u32 = 2;
pub const ROLES: u32 = 3;
pub const BANS: u32 = 4;
pub const COORDINATOR: u32 = 5;
pub const POLICIES: u32 = 6;
pub const INVITES: u32 = 7;
// Subkey 8: reserved.
pub const AUTOMOD: u32 = 9;
pub const ONBOARDING: u32 = 10;
pub const WELCOME: u32 = 11;
/// Subkey 12 held the v1 registry spine. v2 discovers segments from
/// `SegmentAdded` governance entries instead; the index stays
/// reserved so the rest of the table keeps its numbering.
pub const REGISTRY_SPINE_V1: u32 = 12;
// Subkey 13: reserved.
pub const AUDIT_LOG_KEY: u32 = 14;
pub const SUBKEY_COUNT: u32 = 16;
