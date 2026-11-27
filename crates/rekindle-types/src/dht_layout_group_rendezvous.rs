/// The creator's owner subkey, written once with [`OPENED`].
pub const OWNER_SUBKEY: u32 = 0;
/// What the creator writes to [`OWNER_SUBKEY`] at creation: the
/// layout's version.
pub const OPENED: &[u8] = &[1];
/// Owner subkeys in the schema.
pub const OWNER_SUBKEY_COUNT: u16 = 1;
/// One-time writers (= rows = columns): a group's most members
/// (`group_dm::MAX_MEMBERS`).
pub const WRITERS: u8 = 8;
/// Subkeys each writer owns: one per possible recipient.
pub const ROW_LEN: u16 = 8;
/// Total subkeys of the record: 1 + 8 × 8 = 65.
pub const SUBKEY_COUNT: u16 = OWNER_SUBKEY_COUNT + (WRITERS as u16) * ROW_LEN;
/// Largest value a subkey accepts:
/// `min(MAX_SUBKEY_SIZE, MAX_RECORD_DATA_SIZE / subkey_count)` =
/// `min(32,768, 1,048,576 / 65)` = 16,131 bytes (veilid-core 0.5.8
/// `veilid_api/types/dht/derivation.rs:186-189`, constants at
/// `veilid_api/types/dht/mod.rs:36,38`); a group `PreKeyInit` frame
/// (~5.5 KB with its ML-KEM ciphertext) fits.
pub const MAX_SUBKEY_VALUE_LEN: usize = 16_131;

/// The subkey of member `from`'s first contact to member `to` (both
/// slots, `0..WRITERS`): `1 + 8·from + to`. `None` for a slot out of
/// range or a member to itself.
#[must_use]
pub fn subkey(from: u8, to: u8) -> Option<u32> {
    (from < WRITERS && to < WRITERS && from != to).then(|| {
        u32::from(OWNER_SUBKEY_COUNT) + u32::from(from) * u32::from(ROW_LEN) + u32::from(to)
    })
}

/// The subkeys member `to` watches: every other row's entry to it.
#[must_use]
pub fn column(to: u8) -> Vec<u32> {
    (0..WRITERS).filter_map(|from| subkey(from, to)).collect()
}

/// The (row, column) a subkey holds: whose first contact to whom.
#[must_use]
pub fn slots_of(subkey: u32) -> Option<(u8, u8)> {
    let index = subkey.checked_sub(u32::from(OWNER_SUBKEY_COUNT))?;
    let row = u8::try_from(index / u32::from(ROW_LEN)).ok()?;
    let col = u8::try_from(index % u32::from(ROW_LEN)).ok()?;
    (row < WRITERS && row != col).then_some((row, col))
}
