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
    pub const PREKEY_BUNDLE: u32 = 5;
    pub const ROUTE_BLOB: u32 = 6;
    pub const METADATA: u32 = 7;

    /// Strand Relay pool (architecture §13.2 step 3-4): JSON-encoded
    /// `Vec<Vec<u8>>` of opaque relay route blobs from friends, padded
    /// with dummies for size privacy.
    pub const RELAY_POOL: u32 = 8;

    /// Friend request inbox key — the DHT key of this user's friend
    /// inbox record. Published so anyone can discover where to send
    /// requests.
    pub const FRIEND_INBOX_KEY: u32 = 9;

    /// Hex-encoded keypair for the friend inbox. Published so anyone
    /// can open the inbox for writing to submit friend requests.
    pub const FRIEND_INBOX_KEYPAIR: u32 = 10;

    /// Total subkeys to allocate when creating a profile record.
    pub const SUBKEY_COUNT: u32 = 11;

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

/// Subkey layout for the member registry record (SMPL, `o_cnt: 0`).
///
/// Every subkey is a member slot addressed by its raw index. The
/// community-wide entries below predate flat governance and still
/// occupy low indices that now belong to members — see the module
/// header of `rekindle_transport::broadcast::dht::registry`.
pub mod registry {
    pub const MEMBER_INDEX: u32 = 0;
    pub const MEK_VAULT: u32 = 1;
    pub const MODERATION_QUEUE: u32 = 5;
}

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

#[cfg(test)]
mod tests {
    use super::{manifest, profile};

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
            ("PREKEY_BUNDLE", profile::PREKEY_BUNDLE),
            ("ROUTE_BLOB", profile::ROUTE_BLOB),
            ("METADATA", profile::METADATA),
            ("RELAY_POOL", profile::RELAY_POOL),
            ("FRIEND_INBOX_KEY", profile::FRIEND_INBOX_KEY),
            ("FRIEND_INBOX_KEYPAIR", profile::FRIEND_INBOX_KEYPAIR),
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
            profile::PREKEY_BUNDLE,
            profile::ROUTE_BLOB,
            profile::METADATA,
            profile::RELAY_POOL,
            profile::FRIEND_INBOX_KEY,
            profile::FRIEND_INBOX_KEYPAIR,
        ]
        .into_iter()
        .max()
        .expect("non-empty");
        assert_eq!(
            profile::SUBKEY_COUNT,
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
        assert_eq!(profile::PREKEY_BUNDLE, 5);
        assert_eq!(profile::ROUTE_BLOB, 6);
        assert_eq!(profile::METADATA, 7);
        assert_eq!(profile::RELAY_POOL, 8);
        assert_eq!(profile::FRIEND_INBOX_KEY, 9);
        assert_eq!(profile::FRIEND_INBOX_KEYPAIR, 10);
        assert_eq!(profile::SUBKEY_COUNT, 11);
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
