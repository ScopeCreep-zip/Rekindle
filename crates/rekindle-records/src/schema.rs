//! Universal SMPL schema builder — the Q-pid equation.
//!
//! Every DHT record in a v2.0 community (governance, registry, channels)
//! uses the same SMPL structure: `o_cnt: 0`, N member slots, 1 subkey each.
//!
//! `o_cnt: 0` means the creation keypair owns NO writable subkeys.
//! All subkeys belong to declared members. The creation keypair is still
//! needed to reopen the record (it's embedded in the record key hash)
//! but cannot write to any subkey.
//!
//! See architecture doc §4.2 and Veilid developer book §DHT.
//! Verified: `DHTSchema::smpl(0, members)` is valid in veilid-core 0.5.2
//! when members provide at least 1 subkey (total subkeys must be >= 1).

use veilid_core::{BareMemberId, DHTSchema, DHTSchemaSMPLMember, VeilidAPIResult};

/// Maximum members per SMPL record segment.
/// Veilid allows up to 1024 total subkeys. With m_cnt=1 per member,
/// 255 members = 255 subkeys, well within limits.
pub const MAX_MEMBERS_PER_SEGMENT: usize = 255;

/// Build the universal SMPL schema for v2.0 community records.
///
/// - `o_cnt: 0` — no owner subkeys (flat governance, creation keypair discarded).
/// - Each member gets exactly 1 subkey (`m_cnt: 1`).
/// - The `member_public_keys` are raw 32-byte Ed25519 public keys (derived from slot seed).
///
/// For VLD0 crypto kind, `BareMemberId` IS the raw 32-byte public key
/// (MEMBER_ID_LENGTH == VLD0_PUBLIC_KEY_LENGTH == 32, so no hashing occurs).
///
/// # Errors
/// Returns error if members list is empty (need at least 1 subkey) or exceeds 1024.
pub fn community_smpl_schema(member_public_keys: &[[u8; 32]]) -> VeilidAPIResult<DHTSchema> {
    let members: Vec<DHTSchemaSMPLMember> = member_public_keys
        .iter()
        .map(|pubkey| DHTSchemaSMPLMember {
            m_key: BareMemberId::new(pubkey),
            m_cnt: 1,
        })
        .collect();

    DHTSchema::smpl(0, members)
}

/// Build a DFLT schema for the bootstrap pointer record.
///
/// Single subkey, written once at community creation with immutable pointers
/// to the governance and registry records. Used only for discovery.
pub fn bootstrap_dflt_schema() -> VeilidAPIResult<DHTSchema> {
    DHTSchema::dflt(1)
}

/// Build a DFLT schema for the personal cross-device sync record
/// (architecture §28.4). 16 subkeys: 4 active (manifest / read state /
/// preferences / device list) + 12 reserved.
pub fn personal_sync_dflt_schema() -> VeilidAPIResult<DHTSchema> {
    DHTSchema::dflt(16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn community_schema_creates_with_ocnt_zero() {
        let keys: Vec<[u8; 32]> = (0..3u8).map(|i| [i; 32]).collect();
        let schema = community_smpl_schema(&keys).unwrap();
        match schema {
            DHTSchema::SMPL(smpl) => {
                assert_eq!(smpl.o_cnt(), 0, "o_cnt must be 0 for flat governance");
                assert_eq!(smpl.members().len(), 3);
                for m in smpl.members() {
                    assert_eq!(m.m_cnt, 1);
                }
            }
            _ => panic!("expected SMPL schema"),
        }
    }

    #[test]
    fn community_schema_255_members() {
        let keys: Vec<[u8; 32]> = (0..255u16)
            .map(|i| {
                let mut k = [0u8; 32];
                k[0] = (i & 0xFF) as u8;
                k[1] = (i >> 8) as u8;
                k
            })
            .collect();
        let schema = community_smpl_schema(&keys).unwrap();
        match schema {
            DHTSchema::SMPL(smpl) => {
                assert_eq!(smpl.members().len(), 255);
            }
            _ => panic!("expected SMPL schema"),
        }
    }

    #[test]
    fn bootstrap_schema_is_dflt() {
        let schema = bootstrap_dflt_schema().unwrap();
        match schema {
            DHTSchema::DFLT(dflt) => {
                assert_eq!(dflt.o_cnt(), 1);
            }
            _ => panic!("expected DFLT schema"),
        }
    }
}
