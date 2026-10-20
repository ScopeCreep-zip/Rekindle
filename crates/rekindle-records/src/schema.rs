//! The schemas of every record Rekindle creates, as Veilid-free
//! [`RecordSchema`]s (plan C9.1c). The port's Veilid implementation
//! converts and validates them (`rekindle_protocol::dht::pool::dht_schema`);
//! a schema Veilid refuses (no subkeys, or more than 1024) is a
//! `PortError::Dht` from `create`.
//!
//! Community records (governance, registry, channels) share the universal
//! SMPL shape, the Q-pid equation (architecture doc §4.2): `o_cnt: 0`, so
//! the creation keypair writes nothing, and one subkey per member slot.

use rekindle_types::dht_layout as layout;

use crate::port::{RecordSchema, SchemaMember};

/// Maximum members per SMPL record segment.
/// Veilid allows up to 1024 total subkeys. With m_cnt=1 per member,
/// 255 members = 255 subkeys, well within limits.
pub const MAX_MEMBERS_PER_SEGMENT: usize = 255;

/// The universal community SMPL schema: no owner subkeys, one subkey per
/// member, each member key a raw 32-byte Ed25519 public key (for VLD0 the
/// member id is the key itself).
#[must_use]
pub fn community_smpl(member_public_keys: &[[u8; 32]]) -> RecordSchema {
    RecordSchema::Smpl {
        owner_subkeys: 0,
        members: member_public_keys
            .iter()
            .map(|key| SchemaMember {
                key: *key,
                subkeys: 1,
            })
            .collect(),
    }
}

/// A single invite's encrypted `InviteSecrets` record: one owner subkey,
/// so the several-KB payload fits the single-owner budget (architecture §6).
#[must_use]
pub fn invite_secrets() -> RecordSchema {
    RecordSchema::Dflt { owner_subkeys: 1 }
}

/// The personal cross-device sync record (architecture §28.4): 4 active
/// subkeys (manifest, read state, preferences, device list), 12 reserved.
#[must_use]
pub fn personal_sync() -> RecordSchema {
    RecordSchema::Dflt { owner_subkeys: 16 }
}

/// Our profile record.
#[must_use]
pub fn profile() -> RecordSchema {
    RecordSchema::Dflt {
        owner_subkeys: layout::profile::SUBKEY_COUNT,
    }
}

/// Our mailbox record (owned by the identity keypair).
#[must_use]
pub fn mailbox() -> RecordSchema {
    RecordSchema::Dflt {
        owner_subkeys: layout::mailbox::SUBKEY_COUNT,
    }
}

/// A device's prekey record (plan E2.4): the head and the one-time slots,
/// owned by a keypair of its own.
#[must_use]
pub fn prekeys() -> RecordSchema {
    RecordSchema::Dflt {
        owner_subkeys: layout::prekeys::SUBKEY_COUNT,
    }
}

/// A single-subkey record: our friend list, the account header.
#[must_use]
pub fn single() -> RecordSchema {
    RecordSchema::Dflt { owner_subkeys: 1 }
}

/// A community channel record: the channel layout's owner subkeys, then a
/// member page of the layout's size per slot key, in slot order (the slot
/// keys derive from the community's slot seed).
#[must_use]
pub fn channel(slot_public_keys: &[[u8; 32]]) -> RecordSchema {
    RecordSchema::Smpl {
        owner_subkeys: layout::channel::OWNER_SUBKEY_COUNT,
        members: slot_public_keys
            .iter()
            .map(|key| SchemaMember {
                key: *key,
                subkeys: layout::channel::MEMBER_SUBKEY_COUNT,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_size_their_records() {
        assert_eq!(profile().subkey_count(), 9);
        assert_eq!(mailbox().subkey_count(), 1);
        assert_eq!(prekeys().subkey_count(), 101);
        let keys = [[1u8; 32], [2u8; 32]];
        assert_eq!(community_smpl(&keys).subkey_count(), 2);
        assert_eq!(channel(&keys).subkey_count(), 2);
    }
}
