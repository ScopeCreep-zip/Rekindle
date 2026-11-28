/// The inviter's owner subkey, written once with [`OPENED`].
pub const OWNER_SUBKEY: u32 = 0;
/// What the inviter writes to [`OWNER_SUBKEY`] at creation: the layout's
/// version.
pub const OPENED: &[u8] = &[1];
/// Owner subkeys in the schema.
pub const OWNER_SUBKEY_COUNT: u16 = 1;
/// The one-time writer's subkey: the requester's sealed `FriendRequest`
/// frame.
pub const REQUEST: u32 = 1;
/// Subkeys the one-time writer owns.
pub const WRITER_SUBKEY_COUNT: u16 = 1;
/// Total subkeys of the record.
pub const SUBKEY_COUNT: u16 = OWNER_SUBKEY_COUNT + WRITER_SUBKEY_COUNT;
/// Largest value the writer's subkey accepts:
/// `min(MAX_SUBKEY_SIZE, MAX_RECORD_DATA_SIZE / subkey_count)` =
/// `min(32,768, 1,048,576 / 2)` = 32,768 bytes
/// (veilid-core 0.5.8 `veilid_api/types/dht/derivation.rs:186-189` SMPL
/// `max_value_len`, constants at `veilid_api/types/dht/mod.rs:36,38`).
pub const MAX_SUBKEY_VALUE_LEN: usize = 32_768;
