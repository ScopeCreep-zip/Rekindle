//! The group-DM invite payload (architecture §27.2): the MEK is randomly
//! generated and wrapped per recipient. The 2-party `DmInvite` went with
//! the 1:1 SMPL path (plan E2.3).

use serde::{Deserialize, Serialize};

/// Group-DM (3-8 participants) invitation: the SMPL record, the wrapped
/// MEK and the full participant list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroupDmInvite {
    pub record_key: String,
    pub slot_seed: Vec<u8>,
    pub initiator_pseudonym: String,
    /// Participant list: each entry pairs a pseudonym with the SMPL
    /// subkey they will write to and their public key (verification).
    pub participants: Vec<GroupDmParticipant>,
    /// MEK material wrapped for *this specific* recipient with their
    /// X25519 identity public key (architecture §27.2).
    pub wrapped_mek: Vec<u8>,
    /// MEK generation; starts at 0 and increments on rotation.
    pub mek_generation: u32,
}

pub use rekindle_types::member::GroupDmParticipant;
