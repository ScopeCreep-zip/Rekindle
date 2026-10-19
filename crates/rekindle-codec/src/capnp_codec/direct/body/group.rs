//! Group-DM bodies, moved unchanged from the pre-E2.3 1:1 payload
//! (`GroupDmInvite`, and `DmLeave` as `GroupDmLeave`). They stay with the
//! SMPL group path in `src-tauri` and `rekindle-dm` until E2.6 replaces
//! them with the members' sender keys; GroupDmAccept and GroupDmDecline
//! have no body until then.

use crate::capnp_codec::direct::{data, text};
use crate::capnp_codec::CodecError;
use crate::direct_capnp::{group_dm_invite, group_dm_leave};

/// 0x0120 GroupDmInvite: the SMPL record, its slot seed, and the MEK
/// wrapped for this recipient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDmInvite {
    pub record_key: String,
    pub slot_seed: Vec<u8>,
    pub initiator_pseudonym: String,
    /// JSON-encoded `Vec<rekindle_dm::GroupDmParticipant>`.
    pub participants_json: String,
    pub wrapped_mek: Vec<u8>,
    pub mek_generation: u32,
}

impl GroupDmInvite {
    pub(super) fn write(&self, mut b: group_dm_invite::Builder<'_>) {
        b.set_record_key(&self.record_key);
        b.set_slot_seed(&self.slot_seed);
        b.set_initiator_pseudonym(&self.initiator_pseudonym);
        b.set_participants_json(&self.participants_json);
        b.set_wrapped_mek(&self.wrapped_mek);
        b.set_mek_generation(self.mek_generation);
    }

    pub(super) fn read(r: group_dm_invite::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            record_key: text(r.get_record_key())?,
            slot_seed: data(r.get_slot_seed())?,
            initiator_pseudonym: text(r.get_initiator_pseudonym())?,
            participants_json: text(r.get_participants_json())?,
            wrapped_mek: data(r.get_wrapped_mek())?,
            mek_generation: r.get_mek_generation(),
        })
    }
}

/// 0x0123 GroupDmLeave: a member left the group DM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDmLeave {
    pub record_key: String,
}

impl GroupDmLeave {
    pub(super) fn write(&self, mut b: group_dm_leave::Builder<'_>) {
        b.set_record_key(&self.record_key);
    }

    pub(super) fn read(r: group_dm_leave::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            record_key: text(r.get_record_key())?,
        })
    }
}
