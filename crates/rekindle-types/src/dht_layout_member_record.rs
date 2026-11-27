/// The member's per-community card: Cap'n Proto `MemberCard`.
pub const CARD: u32 = 0;
/// The presence beat: Cap'n Proto `PresenceBeat`.
pub const BEAT: u32 = 1;
/// The head of the member's governance op log: `GovHead`.
pub const GOV_HEAD: u32 = 2;
/// The ops the member has seen: `Frontier`, at most
/// [`MAX_FRONTIER_REFS`] `OpRef`s.
pub const FRONTIER: u32 = 3;
/// The member's outbox.
pub const OUTBOX: u32 = 4;
/// The member's history ad: `HistoryAdRecord`.
pub const HISTORY_AD: u32 = 5;
/// The member's routes: `MemberRoutes`, the general private route
/// blob every app_call to the member uses (plan E3.2 S7c.8, E3.4).
pub const ROUTES: u32 = 6;
/// The member's key drop (plan RV2d, D16): the `MekTransfer` envelopes
/// it still owes joiners, each sealed to its joiner's pseudonym, so a
/// joiner whose calls do not arrive takes its key from the record it
/// watches. Only the member writes it (Veilid's owner rule).
pub const KEY_DROP: u32 = 7;
/// Most `OpRef`s a [`FRONTIER`] value carries.
pub const MAX_FRONTIER_REFS: usize = 64;
/// Total subkeys to allocate when creating the record.
pub const SUBKEY_COUNT: u16 = 8;

/// Largest value a subkey of the record accepts:
/// `min(MAX_SUBKEY_SIZE, MAX_RECORD_DATA_SIZE / o_cnt)` =
/// `min(32,768, 1,048,576 / 8)` = 32,768 bytes
/// (veilid-core 0.5.8 `veilid_api/types/dht/derivation.rs:145-148`, with
/// the two constants at `veilid_api/types/dht/mod.rs:36,38`).
pub const MAX_SUBKEY_VALUE_LEN: usize = 32_768;
