//! The group-DM participant (architecture §27.2). The invite itself is
//! the 0x0120 `GroupDmInvite` body (`rekindle_codec::capnp_codec::direct`,
//! plan E2.3), whose `participants_json` lists these; the 2-party
//! `DmInvite` went with the 1:1 SMPL path (plan E2.3).

pub use rekindle_types::member::GroupDmParticipant;
