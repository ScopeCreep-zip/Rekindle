//! DHT record subkey layouts — the single source of truth for both
//! network tracks.
//!
//! The desktop track (`rekindle-protocol`) and the daemon track
//! (`rekindle-transport`) read and write the *same* DHT records, so a
//! subkey index means nothing unless both agree on it. They previously
//! kept private copies of this table and drifted: protocol allocated 9
//! subkeys with index 8 as the Strand Relay pool, while transport
//! allocated 10 with index 8 as the friend-inbox key. A profile written
//! by one and read by the other returned the wrong field entirely.
//!
//! Tier 1 (this crate) is the shared home precisely because both tracks
//! already depend on it and it has no dependencies of its own. Anything
//! that indexes a shared record belongs here — never in a track.

/// Largest `app_message` / `app_call` payload Veilid accepts:
/// `MAX_APP_MESSAGE_MESSAGE_LEN` and `MAX_APP_CALL_Q_MESSAGE_LEN`, both
/// 32,768 bytes (`veilid-core/src/rpc_processor/coders/operations/
/// operation_app_message.rs:3`, `operation_app_call.rs:3-4`, fork
/// 0cd847de). Every encoded envelope a peer sends must fit, so the
/// sender refuses a larger one before Veilid sees it (plan C7.25).
pub const MAX_APP_MESSAGE_LEN: usize = 32_768;

/// Maximum member slots per registry segment.
///
/// Veilid's `DHTSchemaSMPL` has `MAX_MEMBER_COUNT = 256` and
/// `MAX_WRITER_COUNT = 256`. Under the v2.0 `o_cnt: 0` schema the
/// creation keypair owns no subkeys and is NOT counted as a writer
/// (`DHTSchemaSMPL::validate()` only does `writer_count += 1` when
/// `o_cnt > 0`), so 256 member slots would validate.
///
/// We allocate 255 anyway: the value sets Plate Gate segment
/// boundaries, so changing it reshards existing communities. One extra
/// slot is not worth a wire-visible migration.
pub const SLOTS_PER_SEGMENT: u32 = 255;

/// Subkey layout for a user's profile DHT record.
///
/// Indices 0-7 were always agreed. Index 8 is the Strand Relay pool
/// (its original protocol-side meaning, kept so existing desktop-written
/// records keep their semantics); the friend-inbox pair moved to 9/10,
/// which no reader had claimed for anything else.
pub mod profile {
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

    /// Total subkeys to allocate when creating a profile record.
    pub const SUBKEY_COUNT: u16 = 9;

    /// Status byte encoding for [`STATUS`].
    pub const STATUS_ONLINE: u8 = 0;
    pub const STATUS_AWAY: u8 = 1;
    pub const STATUS_BUSY: u8 = 2;
    pub const STATUS_OFFLINE: u8 = 3;
}

/// Subkey layout for the community governance manifest record.
///
/// Both tracks carried identical copies of this table. They happened to
/// agree, but nothing enforced it — the profile and registry tables in
/// the same two files did not.
pub mod manifest {
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
}

// The member registry record (SMPL, `o_cnt: 0`) has **no** layout
// module, and that absence is the v2.0 design rather than an omission.
//
// Every subkey is a member slot addressed by its raw index. The three
// community-wide entries that used to live at 0, 1 and 5 —
// `MEMBER_INDEX`, `MEK_VAULT`, `MODERATION_QUEUE` — were v1.0 owner
// subkeys, and `o_cnt: 0` credentials nobody to write them. Each has
// a v2.0 home: membership is derived by scanning signed presence rows
// (`rekindle_governance_runtime::roster`), the key is distributed
// peer-to-peer (`rekindle-mek-rotation`), and admission is a
// `GovernanceEntry`.

/// Subkey layout for a mailbox record.
pub mod mailbox {
    pub const ROUTE_BLOB: u32 = 0;
    pub const SUBKEY_COUNT: u16 = 1;
}

/// Subkey layout for a channel record (SMPL, `o_cnt: 0`).
pub mod channel {
    pub const HEADER_SUBKEY: u32 = 0;
    /// Zero owner subkeys — the v2.0 universal schema.
    pub const OWNER_SUBKEY_COUNT: u16 = 0;
    pub const MEMBER_SUBKEY_COUNT: u16 = 1;
}

/// Subkey layout for a device's prekey record (plan E2.4): a DFLT(101)
/// record with an owner keypair of its own, named from profile subkey
/// [`profile::PREKEY_RECORDS`]. Subkey 0 is the `PrekeyRecordHead` (the
/// signed prekey and the ML-KEM last-resort key); subkeys 1..=100 each
/// hold one `OpkSlot` (a one-time curve key and a one-time ML-KEM key).
pub mod prekeys {
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
    /// (`veilid-core/src/storage_manager/schema.rs:61-64`, with the two
    /// constants at `storage_manager/types/mod.rs:14-16`, fork 0cd847de).
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
}

/// Subkey layout for a DM participant record (plan E2.5): one DFLT(2)
/// record per participant per conversation, owned by a random keypair only
/// that participant holds. It anchors the participant's append-only chain
/// of sealed DFLT(1) pages (`rekindle_records::chain`).
pub mod dm_participant {
    /// The newest chain page: Cap'n Proto `ChainHead`.
    pub const HEAD: u32 = 0;
    /// The highest retired sequence number: Cap'n Proto `ChainRetire`.
    pub const RETIRE_THROUGH: u32 = 1;
    /// Total subkeys to allocate when creating the record.
    pub const SUBKEY_COUNT: u16 = 2;
}

/// Subkey layout for a friend invite's request inbox (plan E2.5, D17):
/// `SMPL(o_cnt: 1, [oneTimeWriter])`, a single-use capability. The invite
/// carries the one-time writer's keypair, so exactly one requester can
/// write [`request_inbox::REQUEST`]; Veilid refuses any other writer
/// (`veilid-core/src/storage_manager/schema.rs` `check_subkey_value_data`,
/// "wrong writer", fork 0cd847de). The owner subkey is the inviter's: it
/// writes [`request_inbox::OPENED`] there once at creation, which puts the
/// record on the network, so the requester's write finds its descriptor.
pub mod request_inbox {
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
    /// (`veilid-core/src/storage_manager/schema.rs` SMPL `max_value_len`,
    /// constants at `storage_manager/types/mod.rs:14-16`, fork 0cd847de).
    pub const MAX_SUBKEY_VALUE_LEN: usize = 32_768;
}

/// Subkey layout for a community's anchor `A` (plan E3.2, ADR 0011): a
/// DFLT(1) record whose one subkey holds the `GenesisCert`, written once by
/// the anchor's owner `K_A`, whose keypair is then discarded. The community
/// id is `A`'s record key.
pub mod community_anchor {
    /// The `GenesisCert`.
    pub const GENESIS_CERT: u32 = 0;
    /// Total subkeys to allocate when creating the anchor.
    pub const SUBKEY_COUNT: u16 = 1;
}

/// Subkey layout for a member's `MemberRecord` `M(p)` (plan E3.2, D15,
/// ADR 0011): one DFLT(8) record per member per community, owned by a
/// random keypair `K_M(p)` only that member holds.
pub mod member_record {
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
    /// Subkeys 6 and 7: reserved.
    pub const FIRST_RESERVED: u32 = 6;
    /// Most `OpRef`s a [`FRONTIER`] value carries.
    pub const MAX_FRONTIER_REFS: usize = 64;
    /// Total subkeys to allocate when creating the record.
    pub const SUBKEY_COUNT: u16 = 8;

    /// Largest value a subkey of the record accepts:
    /// `min(MAX_SUBKEY_SIZE, MAX_RECORD_DATA_SIZE / o_cnt)` =
    /// `min(32,768, 1,048,576 / 8)` = 32,768 bytes
    /// (`veilid-core/src/storage_manager/schema.rs:61-64`, with the two
    /// constants at `storage_manager/types/mod.rs:14-16`, fork 0cd847de).
    pub const MAX_SUBKEY_VALUE_LEN: usize = 32_768;
}

/// Subkey layout for a community's sealed page (plan E3.2): an op-log,
/// spill or directory page, a DFLT(1) record written once through E2.5's
/// chain primitive (`rekindle_records::chain`), its owner keypair then
/// discarded.
pub mod sealed_page {
    /// The page's content.
    pub const CONTENT: u32 = 0;
    /// Total subkeys to allocate when creating the page.
    pub const SUBKEY_COUNT: u16 = 1;
}

#[cfg(test)]
mod tests {
    use super::{
        community_anchor, dm_participant, manifest, member_record, prekeys, profile, request_inbox,
        sealed_page,
    };

    /// Pins the community anchor, the member record (D15) and the sealed
    /// page: their subkeys are wire-visible to every member.
    #[test]
    fn community_layouts_are_pinned() {
        assert_eq!(community_anchor::GENESIS_CERT, 0);
        assert_eq!(community_anchor::SUBKEY_COUNT, 1);
        assert_eq!(sealed_page::CONTENT, 0);
        assert_eq!(sealed_page::SUBKEY_COUNT, 1);
        let map = [
            member_record::CARD,
            member_record::BEAT,
            member_record::GOV_HEAD,
            member_record::FRONTIER,
            member_record::OUTBOX,
            member_record::HISTORY_AD,
        ];
        assert_eq!(map, [0, 1, 2, 3, 4, 5]);
        assert_eq!(member_record::FIRST_RESERVED, 6);
        assert_eq!(member_record::SUBKEY_COUNT, 8);
        assert_eq!(member_record::MAX_FRONTIER_REFS, 64);
        let cap = usize::min(32_768, 1_048_576 / usize::from(member_record::SUBKEY_COUNT));
        assert_eq!(member_record::MAX_SUBKEY_VALUE_LEN, cap);
    }

    /// Pins the request inbox: the owner subkey, then the writer's, and the
    /// per-subkey cap Veilid derives for two subkeys.
    #[test]
    fn request_inbox_layout_is_pinned() {
        assert_eq!(request_inbox::OWNER_SUBKEY, 0);
        assert_eq!(
            request_inbox::REQUEST,
            u32::from(request_inbox::OWNER_SUBKEY_COUNT)
        );
        assert_eq!(request_inbox::SUBKEY_COUNT, 2);
        let cap = usize::min(32_768, 1_048_576 / usize::from(request_inbox::SUBKEY_COUNT));
        assert_eq!(request_inbox::MAX_SUBKEY_VALUE_LEN, cap);
    }

    /// Pins the prekey record: the head, then exactly `SLOT_COUNT` slots,
    /// and the per-subkey cap Veilid derives for a DFLT(101) record.
    #[test]
    fn prekey_layout_is_pinned() {
        assert_eq!(prekeys::HEAD, 0);
        assert_eq!(prekeys::FIRST_SLOT, 1);
        assert_eq!(prekeys::SLOT_COUNT, 100);
        assert_eq!(
            u32::from(prekeys::SUBKEY_COUNT),
            prekeys::FIRST_SLOT + prekeys::SLOT_COUNT
        );
        // veilid-core: MAX_SUBKEY_SIZE = 32,768, MAX_RECORD_DATA_SIZE = 1 MiB.
        let cap = usize::min(32_768, 1_048_576 / usize::from(prekeys::SUBKEY_COUNT));
        assert_eq!(prekeys::MAX_SUBKEY_VALUE_LEN, cap);
        assert!(!prekeys::is_slot(prekeys::HEAD));
        assert!(prekeys::is_slot(prekeys::slot_subkey(0)));
        assert!(prekeys::is_slot(prekeys::slot_subkey(
            prekeys::SLOT_COUNT - 1
        )));
        assert!(!prekeys::is_slot(prekeys::slot_subkey(prekeys::SLOT_COUNT)));
    }

    /// Pins the DM participant record: the head, then the retire mark.
    #[test]
    fn dm_participant_layout_is_pinned() {
        assert_eq!(dm_participant::HEAD, 0);
        assert_eq!(dm_participant::RETIRE_THROUGH, 1);
        assert_eq!(
            u32::from(dm_participant::SUBKEY_COUNT),
            dm_participant::RETIRE_THROUGH + 1
        );
    }

    /// Every profile subkey must be distinct. The drift this module
    /// exists to prevent was exactly a collision — two meanings on
    /// index 8 — so assert it directly rather than trusting review.
    #[test]
    fn profile_subkeys_are_unique() {
        let all = [
            ("DISPLAY_NAME", profile::DISPLAY_NAME),
            ("STATUS_MESSAGE", profile::STATUS_MESSAGE),
            ("STATUS", profile::STATUS),
            ("AVATAR", profile::AVATAR),
            ("GAME_INFO", profile::GAME_INFO),
            ("PREKEY_RECORDS", profile::PREKEY_RECORDS),
            ("ROUTE_BLOB", profile::ROUTE_BLOB),
            ("METADATA", profile::METADATA),
            ("RELAY_POOL", profile::RELAY_POOL),
        ];
        for (i, (name_a, a)) in all.iter().enumerate() {
            for (name_b, b) in &all[i + 1..] {
                assert_ne!(a, b, "{name_a} and {name_b} share subkey index {a}");
            }
        }
    }

    /// Allocation must cover every declared subkey — an index at or
    /// past `SUBKEY_COUNT` is unwritable on a freshly created record.
    #[test]
    fn subkey_count_covers_every_index() {
        let highest = [
            profile::DISPLAY_NAME,
            profile::STATUS_MESSAGE,
            profile::STATUS,
            profile::AVATAR,
            profile::GAME_INFO,
            profile::PREKEY_RECORDS,
            profile::ROUTE_BLOB,
            profile::METADATA,
            profile::RELAY_POOL,
        ]
        .into_iter()
        .max()
        .expect("non-empty");
        assert_eq!(
            u32::from(profile::SUBKEY_COUNT),
            highest + 1,
            "SUBKEY_COUNT must be one past the highest index"
        );
    }

    /// Pins the wire layout. Changing any of these renumbers a live DHT
    /// record: every peer must agree, so it requires a SCHEMA_VERSION
    /// bump (src-tauri/src/db.rs) and a deliberate migration — not an
    /// incidental edit.
    #[test]
    fn profile_layout_is_pinned() {
        assert_eq!(profile::DISPLAY_NAME, 0);
        assert_eq!(profile::STATUS_MESSAGE, 1);
        assert_eq!(profile::STATUS, 2);
        assert_eq!(profile::AVATAR, 3);
        assert_eq!(profile::GAME_INFO, 4);
        assert_eq!(profile::PREKEY_RECORDS, 5);
        assert_eq!(profile::ROUTE_BLOB, 6);
        assert_eq!(profile::METADATA, 7);
        assert_eq!(profile::RELAY_POOL, 8);
        assert_eq!(profile::SUBKEY_COUNT, 9);
    }

    /// The manifest table is duplicated nowhere now, but its indices are
    /// wire-visible: subkey 8, 12 and 13 are deliberately absent
    /// (reserved / retired), so a "tidy up the gaps" edit would renumber
    /// live records.
    #[test]
    fn manifest_layout_is_pinned() {
        assert_eq!(manifest::METADATA, 0);
        assert_eq!(manifest::CHANNELS, 1);
        assert_eq!(manifest::CATEGORIES, 2);
        assert_eq!(manifest::ROLES, 3);
        assert_eq!(manifest::BANS, 4);
        assert_eq!(manifest::COORDINATOR, 5);
        assert_eq!(manifest::POLICIES, 6);
        assert_eq!(manifest::INVITES, 7);
        assert_eq!(manifest::AUTOMOD, 9);
        assert_eq!(manifest::ONBOARDING, 10);
        assert_eq!(manifest::WELCOME, 11);
        assert_eq!(manifest::REGISTRY_SPINE_V1, 12);
        assert_eq!(manifest::AUDIT_LOG_KEY, 14);
        assert_eq!(manifest::SUBKEY_COUNT, 16);
    }
}
