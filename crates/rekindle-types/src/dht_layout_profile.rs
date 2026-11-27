pub const DISPLAY_NAME: u32 = 0;
pub const STATUS_MESSAGE: u32 = 1;
pub const STATUS: u32 = 2;
pub const AVATAR: u32 = 3;
pub const GAME_INFO: u32 = 4;
/// The device list naming each device's prekey record
/// ([`super::prekeys`]): Cap'n Proto `PrekeyRecordList` (plan E2.4).
pub const PREKEY_RECORDS: u32 = 5;
pub const ROUTE_BLOB: u32 = 6;
pub const METADATA: u32 = 7;

/// Strand Relay pool (architecture §13.2 step 3-4): JSON-encoded
/// `Vec<Vec<u8>>` of opaque relay route blobs from friends, padded
/// with dummies for size privacy.
pub const RELAY_POOL: u32 = 8;
/// The record key of our friend code's current request inbox (plan
/// RV2e): its writer is derived from the code's token, so only the
/// code's holders can write it; rolled after each request lands.
pub const CODE_INBOX: u32 = 9;

/// Total subkeys to allocate when creating a profile record.
pub const SUBKEY_COUNT: u16 = 10;

/// Status byte encoding for [`STATUS`].
pub const STATUS_ONLINE: u8 = 0;
pub const STATUS_AWAY: u8 = 1;
pub const STATUS_BUSY: u8 = 2;
pub const STATUS_OFFLINE: u8 = 3;
