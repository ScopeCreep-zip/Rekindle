//! The creator-set round's bodies (plan E3.2 S7s.3, decisions (16), (17),
//! (20)): the creator's invite to a friend to co-create a reserved
//! community, the friend's signed acceptance or decline, the creator's
//! withdrawal, and the invite link a listed co-creator joins with.

use crate::capnp_codec::CodecError;
use crate::direct_capnp::{
    creator_accept, creator_decline, creator_invite, creator_join, creator_withdrawn,
};
use rekindle_types::key_format;

use super::super::{fixed, record_key, text};

/// A community name a peer wrote ([`key_format::name`], at most
/// [`key_format::MAX_NAME_LEN`] characters).
fn community_name(
    field: Result<capnp::text::Reader<'_>, capnp::Error>,
) -> Result<String, CodecError> {
    let s = text(field)?;
    key_format::name(&s, key_format::MAX_NAME_LEN)
        .map_err(|e| CodecError::Deserialization(format!("community name: {e}")))?;
    Ok(s)
}

/// 0x0190 CreatorInvite: co-create `community_name`, reserved at `anchor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatorInvite {
    pub anchor: String,
    pub community_name: String,
}

impl CreatorInvite {
    pub(super) fn write(&self, mut b: creator_invite::Builder<'_>) {
        b.set_anchor(&self.anchor);
        b.set_community_name(&self.community_name);
    }

    pub(super) fn read(r: creator_invite::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            anchor: record_key(r.get_anchor(), "creator invite anchor")?,
            community_name: community_name(r.get_community_name())?,
        })
    }
}

/// 0x0191 CreatorAccept: our pseudonym for `anchor`, signed by it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatorAccept {
    pub anchor: String,
    pub pseudonym: [u8; 32],
    pub signature: [u8; 64],
}

impl CreatorAccept {
    pub(super) fn write(&self, mut b: creator_accept::Builder<'_>) {
        b.set_anchor(&self.anchor);
        b.set_pseudonym(&self.pseudonym);
        b.set_signature(&self.signature);
    }

    pub(super) fn read(r: creator_accept::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            anchor: record_key(r.get_anchor(), "creator accept anchor")?,
            pseudonym: fixed(r.get_pseudonym(), "creator accept pseudonym")?,
            signature: fixed(r.get_signature(), "creator accept signature")?,
        })
    }
}

/// 0x0192 CreatorDecline: we decline co-creating `anchor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatorDecline {
    pub anchor: String,
}

impl CreatorDecline {
    pub(super) fn write(&self, mut b: creator_decline::Builder<'_>) {
        b.set_anchor(&self.anchor);
    }

    pub(super) fn read(r: creator_decline::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            anchor: record_key(r.get_anchor(), "creator decline anchor")?,
        })
    }
}

/// 0x0193 CreatorWithdrawn: the creator removed us from, or abandoned, the
/// creation of `anchor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatorWithdrawn {
    pub anchor: String,
}

impl CreatorWithdrawn {
    pub(super) fn write(&self, mut b: creator_withdrawn::Builder<'_>) {
        b.set_anchor(&self.anchor);
    }

    pub(super) fn read(r: creator_withdrawn::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            anchor: record_key(r.get_anchor(), "creator withdrawn anchor")?,
        })
    }
}

/// 0x0194 CreatorJoin: `anchor`'s genesis lists us; join it with `invite`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatorJoin {
    pub anchor: String,
    pub invite: String,
}

impl CreatorJoin {
    pub(super) fn write(&self, mut b: creator_join::Builder<'_>) {
        b.set_anchor(&self.anchor);
        b.set_invite(&self.invite);
    }

    /// The invite must be a full invite link to `anchor` itself.
    pub(super) fn read(r: creator_join::Reader<'_>) -> Result<Self, CodecError> {
        let anchor = record_key(r.get_anchor(), "creator join anchor")?;
        let invite = text(r.get_invite())?;
        let link = rekindle_types::invite::InviteLink::parse(&invite)
            .map_err(|e| CodecError::Deserialization(format!("creator join invite: {e}")))?;
        if link.anchor.as_str() != anchor {
            return Err(CodecError::Deserialization(
                "creator join invite names another community".into(),
            ));
        }
        Ok(Self { anchor, invite })
    }
}

#[cfg(test)]
#[path = "creator_tests.rs"]
mod tests;
