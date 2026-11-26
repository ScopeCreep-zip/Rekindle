/// The head: `PrekeyRecordHead`.
pub const HEAD: u32 = 0;
/// The first one-time slot: `OpkSlot`.
pub const FIRST_SLOT: u32 = 1;
/// One-time slots in the record.
pub const SLOT_COUNT: u32 = 100;
/// Total subkeys to allocate when creating the record.
pub const SUBKEY_COUNT: u16 = 101;

/// Largest value an owner subkey of the record accepts:
/// `min(MAX_SUBKEY_SIZE, MAX_RECORD_DATA_SIZE / o_cnt)` =
/// `min(32,768, 1,048,576 / 101)` = 10,381 bytes
/// (veilid-core 0.5.8 `veilid_api/types/dht/derivation.rs:145-148`, with
/// the two constants at `veilid_api/types/dht/mod.rs:36,38`).
/// Every head, slot and device list must fit.
pub const MAX_SUBKEY_VALUE_LEN: usize = 10_381;

/// The subkey holding one-time slot `index` (0-based).
#[must_use]
pub const fn slot_subkey(index: u32) -> u32 {
    FIRST_SLOT + index
}

/// Whether `subkey` is a one-time slot.
#[must_use]
pub const fn is_slot(subkey: u32) -> bool {
    subkey >= FIRST_SLOT && subkey < FIRST_SLOT + SLOT_COUNT
}
