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
        member_record::ROUTES,
        member_record::KEY_DROP,
    ];
    assert_eq!(map, [0, 1, 2, 3, 4, 5, 6, 7]);
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

/// Pins the DM participant record: the head alone, the retire mark in it.
#[test]
fn dm_participant_layout_is_pinned() {
    assert_eq!(dm_participant::HEAD, 0);
    assert_eq!(
        u32::from(dm_participant::SUBKEY_COUNT),
        dm_participant::HEAD + 1
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
        profile::CODE_INBOX,
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
    // The friend code's write slot (plan RV2e, SCHEMA_VERSION 159).
    assert_eq!(profile::CODE_INBOX, 9);
    assert_eq!(profile::SUBKEY_COUNT, 10);
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
