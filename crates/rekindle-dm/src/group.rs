//! A group's signed roster, the creator's `GroupDmInvite`, and the epoch
//! rules of the members' sending keys (plan E2.6).
//!
//! The creator is the roster's first member: the invite's signature is
//! theirs over [`GroupDmInvite::signing_bytes`] (which carries
//! `GROUP_DM_INVITE` and the roster in its order), and an invite is only
//! taken from the pairwise session of that creator.

use rekindle_codec::capnp_codec::direct::{GroupDmInvite, GroupId};
use rekindle_secrets::group_dm::GroupSenderKey;
use rekindle_types::group_dm::{GroupDmRosterMember, MAX_MEMBERS, MAX_NAME_CHARS, MIN_MEMBERS};
use rekindle_utils::RandomSource;

use crate::error::DmError;

/// A group's members, us included: unique identity keys, each with its
/// profile and participant records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roster {
    members: Vec<GroupDmRosterMember>,
}

fn is_identity(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn check_members(members: &[GroupDmRosterMember]) -> Result<(), DmError> {
    if members.len() > MAX_MEMBERS {
        return Err(DmError::InvalidRoster(format!(
            "{} members, at most {MAX_MEMBERS}",
            members.len()
        )));
    }
    for (i, m) in members.iter().enumerate() {
        if !is_identity(&m.identity) {
            return Err(DmError::InvalidRoster(format!(
                "member {i} is not an identity key"
            )));
        }
        if m.profile_record_key.is_empty() || m.participant_record_key.is_empty() {
            return Err(DmError::InvalidRoster(format!(
                "member {i} lacks a record key"
            )));
        }
        let repeated = members[..i].iter().any(|o| {
            o.identity == m.identity || o.participant_record_key == m.participant_record_key
        });
        if repeated {
            return Err(DmError::InvalidRoster(format!("member {i} is repeated")));
        }
    }
    Ok(())
}

impl Roster {
    /// A roster to sign or as signed: 3-8 unique members.
    ///
    /// # Errors
    /// [`DmError::InvalidRoster`].
    pub fn new(members: Vec<GroupDmRosterMember>) -> Result<Self, DmError> {
        if members.len() < MIN_MEMBERS {
            return Err(DmError::InvalidRoster(format!(
                "{} members, at least {MIN_MEMBERS}",
                members.len()
            )));
        }
        check_members(&members)?;
        Ok(Self { members })
    }

    /// A stored roster, which may have shrunk below three by leaves.
    ///
    /// # Errors
    /// [`DmError::InvalidRoster`] (empty, past eight, or a repeat).
    pub fn from_stored(members: Vec<GroupDmRosterMember>) -> Result<Self, DmError> {
        if members.is_empty() {
            return Err(DmError::InvalidRoster("no members".into()));
        }
        check_members(&members)?;
        Ok(Self { members })
    }

    #[must_use]
    pub fn members(&self) -> &[GroupDmRosterMember] {
        &self.members
    }

    /// The creator (the first member).
    #[must_use]
    pub fn creator(&self) -> &GroupDmRosterMember {
        &self.members[0]
    }

    #[must_use]
    pub fn member(&self, identity: &str) -> Option<&GroupDmRosterMember> {
        self.members.iter().find(|m| m.identity == identity)
    }

    #[must_use]
    pub fn contains(&self, identity: &str) -> bool {
        self.member(identity).is_some()
    }

    /// Every member's identity but `me`.
    #[must_use]
    pub fn others(&self, me: &str) -> Vec<String> {
        self.members
            .iter()
            .filter(|m| m.identity != me)
            .map(|m| m.identity.clone())
            .collect()
    }

    /// The roster without `identity` (a leave or a decline); `None` when
    /// it was not on it.
    #[must_use]
    pub fn without(&self, identity: &str) -> Option<Self> {
        if !self.contains(identity) || self.members.len() == 1 {
            return None;
        }
        let members = self
            .members
            .iter()
            .filter(|m| m.identity != identity)
            .cloned()
            .collect();
        Some(Self { members })
    }

    #[must_use]
    pub fn into_members(self) -> Vec<GroupDmRosterMember> {
        self.members
    }
}

/// A group's name: not empty, at most [`MAX_NAME_CHARS`] characters.
///
/// # Errors
/// [`DmError::InvalidName`].
pub fn check_name(name: &str) -> Result<(), DmError> {
    let chars = name.chars().count();
    if chars == 0 || name.trim().is_empty() {
        return Err(DmError::InvalidName("empty".into()));
    }
    if chars > MAX_NAME_CHARS {
        return Err(DmError::InvalidName(format!(
            "{chars} characters, at most {MAX_NAME_CHARS}"
        )));
    }
    Ok(())
}

/// A new group's id: 16 random bytes.
#[must_use]
pub fn new_group_id(rng: &dyn RandomSource) -> GroupId {
    let mut id = [0u8; 16];
    rng.fill(&mut id);
    id
}

/// A group id as stored and shown: lowercase hex.
#[must_use]
pub fn group_id_hex(id: &GroupId) -> String {
    hex::encode(id)
}

/// The creator's invite: the roster (the creator first), the name and the
/// creator's sending key, signed by `sign` (the creator's identity key)
/// over [`GroupDmInvite::signing_bytes`].
pub fn build_invite(
    group_id: GroupId,
    name: &str,
    roster: &Roster,
    creator_key: &GroupSenderKey,
    sign: impl FnOnce(&[u8]) -> [u8; 64],
) -> GroupDmInvite {
    let mut invite = GroupDmInvite {
        group_id,
        name: name.to_owned(),
        roster: roster.members().to_vec(),
        creator_sender_key: *creator_key.as_bytes(),
        epoch: creator_key.epoch(),
        signature: [0; 64],
    };
    invite.signature = sign(&invite.signing_bytes());
    invite
}

/// Verify an invite that arrived over `from`'s pairwise session: its
/// roster is valid, `from` is its creator, and the signature is the
/// creator's identity key over the signed bytes.
///
/// # Errors
/// [`DmError::InvalidRoster`], [`DmError::InvalidName`],
/// [`DmError::NotOnRoster`] (`from` is not the creator), or
/// [`DmError::BadSignature`].
pub fn verify_invite(invite: &GroupDmInvite, from: &str) -> Result<Roster, DmError> {
    let roster = Roster::new(invite.roster.clone())?;
    check_name(&invite.name)?;
    if roster.creator().identity != from {
        return Err(DmError::NotOnRoster(from.to_owned()));
    }
    let mut creator = [0u8; 32];
    hex::decode_to_slice(&roster.creator().identity, &mut creator)
        .map_err(|_| DmError::InvalidRoster("creator identity".into()))?;
    rekindle_secrets::sign::verify_signature(&creator, &invite.signing_bytes(), &invite.signature)
        .map_err(|_| DmError::BadSignature)?;
    Ok(roster)
}

/// What a received sending key does to the one we hold for its member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyUpdate {
    /// The first key, or a newer epoch: store it.
    Store,
    /// The key we already hold, again.
    Duplicate,
}

/// The epoch rule for a member's keys: epochs only move up. An older epoch
/// than the one held is refused, and so is a different key at the same
/// epoch.
///
/// # Errors
/// [`DmError::StaleEpoch`] or [`DmError::ConflictingKey`].
pub fn key_update(
    held: Option<&GroupSenderKey>,
    incoming: &GroupSenderKey,
) -> Result<KeyUpdate, DmError> {
    let Some(held) = held else {
        return Ok(KeyUpdate::Store);
    };
    match incoming.epoch().cmp(&held.epoch()) {
        std::cmp::Ordering::Greater => Ok(KeyUpdate::Store),
        std::cmp::Ordering::Equal if incoming == held => Ok(KeyUpdate::Duplicate),
        std::cmp::Ordering::Equal => Err(DmError::ConflictingKey(held.epoch())),
        std::cmp::Ordering::Less => Err(DmError::StaleEpoch {
            got: incoming.epoch(),
            current: held.epoch(),
        }),
    }
}

/// The epoch rule for a frame: it must be at the sender's current epoch.
/// An older one is refused; a newer one waits for that sender's new key.
///
/// # Errors
/// [`DmError::StaleEpoch`] or [`DmError::KeyMissing`].
pub fn check_frame_epoch(member: &str, frame_epoch: u32, current: u32) -> Result<(), DmError> {
    match frame_epoch.cmp(&current) {
        std::cmp::Ordering::Equal => Ok(()),
        std::cmp::Ordering::Less => Err(DmError::StaleEpoch {
            got: frame_epoch,
            current,
        }),
        std::cmp::Ordering::Greater => Err(DmError::KeyMissing {
            member: member.to_owned(),
            epoch: frame_epoch,
        }),
    }
}

#[cfg(test)]
#[path = "group/tests.rs"]
pub(crate) mod tests;
