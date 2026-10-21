//! Group-DM bodies (plan E2.6): 3-8 members, no governance, sender-owned
//! keys. Each member holds a random group sending key of its own and sends
//! it to every other member over their pairwise session (0x0120 from the
//! creator, 0x0121 from each joiner); content (0x0124) is sealed under the
//! sender's key and written to the sender's group chain. Signing, sealing
//! and verifying are `rekindle-dm`'s (slice G2); these are the bytes.

use rekindle_types::group_dm::{RosterMember, MAX_MEMBERS, MAX_NAME_CHARS, MIN_MEMBERS};

use crate::capnp_codec::direct::{bounded_text, data, fixed, list_len, public_key, record_key};
use crate::capnp_codec::{capnp_err, CodecError};
use crate::direct_capnp::{
    group_content, group_dm_accept, group_dm_decline, group_dm_invite, group_dm_leave,
};

/// A group's id: 16 random bytes its creator mints.
pub type GroupId = [u8; 16];

/// 0x0120 GroupDmInvite, inside the ratchet to each invited member: the
/// signed roster and the creator's sending key.
#[derive(Clone, PartialEq, Eq)]
pub struct GroupDmInvite {
    pub group_id: GroupId,
    pub name: String,
    /// Every member, the creator included.
    pub roster: Vec<RosterMember>,
    pub creator_sender_key: [u8; 32],
    /// The creator's key epoch.
    pub epoch: u32,
    /// The creator's identity signature over [`Self::signing_bytes`].
    pub signature: [u8; 64],
}

/// The key is never printed.
impl std::fmt::Debug for GroupDmInvite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupDmInvite")
            .field("group_id", &hex::encode(self.group_id))
            .field("name", &self.name)
            .field("roster", &self.roster)
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

/// Append `bytes` with its length (u32 big-endian) in front.
fn put_len_prefixed(out: &mut Vec<u8>, bytes: &[u8]) {
    let len = u32::try_from(bytes.len()).expect("a group invite field fits a u32 length");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
}

impl GroupDmInvite {
    /// The bytes the creator signs: `GROUP_DM_INVITE ‖ group_id ‖ name ‖
    /// roster ‖ epoch`, every variable field length-prefixed, the roster in
    /// its order. The sending key is not signed: the pairwise session
    /// already authenticates it to its one recipient, and a signature over
    /// it would make the key a transferable artifact. What admission reads
    /// (who is on the roster, under which records) is signed.
    #[must_use]
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut out = rekindle_types::domains::GROUP_DM_INVITE.as_bytes().to_vec();
        out.extend_from_slice(&self.group_id);
        put_len_prefixed(&mut out, self.name.as_bytes());
        out.extend_from_slice(&list_len(&self.roster).to_be_bytes());
        for member in &self.roster {
            put_len_prefixed(&mut out, member.identity.as_bytes());
            put_len_prefixed(&mut out, member.profile_record_key.as_bytes());
            put_len_prefixed(&mut out, member.participant_record_key.as_bytes());
        }
        out.extend_from_slice(&self.epoch.to_be_bytes());
        out
    }

    pub(super) fn write(&self, mut b: group_dm_invite::Builder<'_>) {
        b.set_group_id(&self.group_id);
        b.set_name(&self.name);
        let mut roster = b.reborrow().init_roster(list_len(&self.roster));
        for (i, member) in self.roster.iter().enumerate() {
            let mut m = roster
                .reborrow()
                .get(u32::try_from(i).expect("a roster index fits a u32"));
            m.set_identity(&member.identity);
            m.set_profile_record_key(&member.profile_record_key);
            m.set_participant_record_key(&member.participant_record_key);
        }
        b.set_creator_sender_key(&self.creator_sender_key);
        b.set_epoch(self.epoch);
        b.set_signature(&self.signature);
    }

    /// A roster outside 3-8 members, a member that is not an identity key
    /// with two record keys, or an empty name or one past its bound is
    /// malformed.
    pub(super) fn read(r: group_dm_invite::Reader<'_>) -> Result<Self, CodecError> {
        let list = r.get_roster().map_err(|e| capnp_err(&e))?;
        let count = list.len() as usize;
        if !(MIN_MEMBERS..=MAX_MEMBERS).contains(&count) {
            return Err(CodecError::Deserialization(format!(
                "group roster of {count} members, want {MIN_MEMBERS}-{MAX_MEMBERS}"
            )));
        }
        let roster = list
            .iter()
            .map(|m| {
                Ok(RosterMember {
                    identity: public_key(m.get_identity(), "roster identity")?,
                    profile_record_key: record_key(
                        m.get_profile_record_key(),
                        "roster profile record key",
                    )?,
                    participant_record_key: record_key(
                        m.get_participant_record_key(),
                        "roster participant record key",
                    )?,
                })
            })
            .collect::<Result<Vec<_>, CodecError>>()?;
        let name = bounded_text(r.get_name(), MAX_NAME_CHARS, "group name")?;
        if name.is_empty() {
            return Err(CodecError::Deserialization("empty group name".into()));
        }
        Ok(Self {
            group_id: fixed(r.get_group_id(), "group id")?,
            name,
            roster,
            creator_sender_key: fixed(r.get_creator_sender_key(), "creator sender key")?,
            epoch: r.get_epoch(),
            signature: fixed(r.get_signature(), "group invite signature")?,
        })
    }
}

/// 0x0121 GroupDmAccept, inside the ratchet to each member: we joined, and
/// our sending key of `epoch`.
#[derive(Clone, PartialEq, Eq)]
pub struct GroupDmAccept {
    pub group_id: GroupId,
    pub epoch: u32,
    pub sender_key: [u8; 32],
}

/// The key is never printed.
impl std::fmt::Debug for GroupDmAccept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupDmAccept")
            .field("group_id", &hex::encode(self.group_id))
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

impl GroupDmAccept {
    pub(super) fn write(&self, mut b: group_dm_accept::Builder<'_>) {
        b.set_group_id(&self.group_id);
        b.set_epoch(self.epoch);
        b.set_sender_key(&self.sender_key);
    }

    pub(super) fn read(r: group_dm_accept::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            group_id: fixed(r.get_group_id(), "group id")?,
            epoch: r.get_epoch(),
            sender_key: fixed(r.get_sender_key(), "group sender key")?,
        })
    }
}

/// 0x0122 GroupDmDecline, inside the ratchet to the creator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDmDecline {
    pub group_id: GroupId,
}

impl GroupDmDecline {
    pub(super) fn write(&self, mut b: group_dm_decline::Builder<'_>) {
        b.set_group_id(&self.group_id);
    }

    pub(super) fn read(r: group_dm_decline::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            group_id: fixed(r.get_group_id(), "group id")?,
        })
    }
}

/// 0x0123 GroupDmLeave, inside the ratchet to each member: we left at our
/// key's `epoch`; every remaining member rotates its own key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDmLeave {
    pub group_id: GroupId,
    pub epoch: u32,
}

impl GroupDmLeave {
    pub(super) fn write(&self, mut b: group_dm_leave::Builder<'_>) {
        b.set_group_id(&self.group_id);
        b.set_epoch(self.epoch);
    }

    pub(super) fn read(r: group_dm_leave::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            group_id: fixed(r.get_group_id(), "group id")?,
            epoch: r.get_epoch(),
        })
    }
}

/// 0x0124 GroupContent: a content kind or typing, sealed under the
/// sender's sending key of `epoch` (`GROUP_DM_CONTENT`), written once to
/// the sender's group chain.
#[derive(Clone, PartialEq, Eq)]
pub struct GroupContent {
    pub group_id: GroupId,
    pub epoch: u32,
    pub sealed: Vec<u8>,
}

/// The ciphertext is shown by its length only.
impl std::fmt::Debug for GroupContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupContent")
            .field("group_id", &hex::encode(self.group_id))
            .field("epoch", &self.epoch)
            .field("sealed_len", &self.sealed.len())
            .finish()
    }
}

impl GroupContent {
    pub(super) fn write(&self, mut b: group_content::Builder<'_>) {
        b.set_group_id(&self.group_id);
        b.set_epoch(self.epoch);
        b.set_sealed(&self.sealed);
    }

    /// An empty ciphertext is malformed.
    pub(super) fn read(r: group_content::Reader<'_>) -> Result<Self, CodecError> {
        let sealed = data(r.get_sealed())?;
        if sealed.is_empty() {
            return Err(CodecError::Deserialization("empty group content".into()));
        }
        Ok(Self {
            group_id: fixed(r.get_group_id(), "group id")?,
            epoch: r.get_epoch(),
            sealed,
        })
    }
}
