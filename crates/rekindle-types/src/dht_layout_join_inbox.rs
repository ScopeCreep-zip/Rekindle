//! One owner marker followed by a bounded range for sealed join locators.

/// The inviter publishes the record at this subkey.
pub const OWNER_SUBKEY: u32 = 0;
/// The inbox layout version, written at creation.
pub const OPENED: &[u8] = &[1];
/// Subkeys held exclusively by the inviter.
pub const OWNER_SUBKEY_COUNT: u16 = 1;
/// The first member-writable subkey.
pub const FIRST_SLOT: u32 = 1;
/// The complete sealed locator, including encapsulated key and tag.
pub const MAX_ENTRY_LEN: usize = 512;

/// An invite allocates one slot per use.
pub fn valid_slots(slots: u8) -> bool {
    slots > 0 && u32::from(slots) <= crate::invite::MAX_USES_PER_INVITE
}

/// The writer's slots in a validated inbox.
pub fn slots(count: u8) -> std::ops::Range<u32> {
    FIRST_SLOT..FIRST_SLOT + u32::from(count)
}
