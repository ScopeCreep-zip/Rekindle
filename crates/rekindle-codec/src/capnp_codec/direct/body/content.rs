//! Content bodies: text, its edits, deletes and reactions, read receipts
//! and typing.

use rekindle_types::key_format::{self, MAX_REACTION_LEN};

use crate::capnp_codec::direct::{fixed, message_body, text, ClientId};
use crate::capnp_codec::{capnp_err, CodecError};
use crate::direct_capnp::{dm_ack, dm_delete, dm_edit, dm_reaction, dm_text, read_up_to, typing};

/// The most entries one `DmAck` names: one coalescing window's worth.
pub const MAX_DM_ACK_SEQS: usize = 256;

/// 0x0004 DmAck: we hold these entries of the peer's DM stream:
/// every one through `through` (cumulative; 0 none) and `stream_seqs`
/// past it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmAck {
    pub stream_seqs: Vec<u64>,
    pub through: u64,
}

impl DmAck {
    pub(super) fn write(&self, mut b: dm_ack::Builder<'_>) {
        b.set_through(self.through);
        let mut seqs =
            b.init_stream_seqs(u32::try_from(self.stream_seqs.len()).unwrap_or(u32::MAX));
        for (i, seq) in (0u32..).zip(&self.stream_seqs) {
            seqs.set(i, *seq);
        }
    }

    pub(super) fn read(r: dm_ack::Reader<'_>) -> Result<Self, CodecError> {
        let seqs = r.get_stream_seqs().map_err(|e| capnp_err(&e))?;
        if seqs.len() as usize > MAX_DM_ACK_SEQS {
            return Err(CodecError::Deserialization(format!(
                "DmAck names {} entries (cap {MAX_DM_ACK_SEQS})",
                seqs.len()
            )));
        }
        Ok(Self {
            stream_seqs: seqs.iter().collect(),
            through: r.get_through(),
        })
    }
}

/// 0x0100 DmText.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmText {
    pub client_id: ClientId,
    pub body: String,
    /// The parent's client id, when this is a reply.
    pub reply_to: Option<ClientId>,
    /// The disappearing-message timer; 0 = none.
    pub expire_seconds: u32,
}

impl DmText {
    pub(super) fn write(&self, mut b: dm_text::Builder<'_>) {
        b.set_client_id(&self.client_id);
        b.set_body(&self.body);
        if let Some(parent) = &self.reply_to {
            b.set_reply_to(parent);
        }
        b.set_expire_seconds(self.expire_seconds);
    }

    pub(super) fn read(r: dm_text::Reader<'_>) -> Result<Self, CodecError> {
        let reply_to = if r.get_reply_to().map_err(|e| capnp_err(&e))?.is_empty() {
            None
        } else {
            Some(fixed(r.get_reply_to(), "reply_to")?)
        };
        Ok(Self {
            client_id: fixed(r.get_client_id(), "client id")?,
            body: message_body(r.get_body())?,
            reply_to,
            expire_seconds: r.get_expire_seconds(),
        })
    }
}

/// 0x0110 DmEdit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmEdit {
    pub target_client_id: ClientId,
    pub body: String,
    /// The author's clock, milliseconds.
    pub edited_at: u64,
}

impl DmEdit {
    pub(super) fn write(&self, mut b: dm_edit::Builder<'_>) {
        b.set_target_client_id(&self.target_client_id);
        b.set_body(&self.body);
        b.set_edited_at(self.edited_at);
    }

    pub(super) fn read(r: dm_edit::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            target_client_id: fixed(r.get_target_client_id(), "edit target")?,
            body: message_body(r.get_body())?,
            edited_at: r.get_edited_at(),
        })
    }
}

/// 0x0111 DmDelete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmDelete {
    pub target_client_id: ClientId,
}

impl DmDelete {
    pub(super) fn write(&self, mut b: dm_delete::Builder<'_>) {
        b.set_target_client_id(&self.target_client_id);
    }

    pub(super) fn read(r: dm_delete::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            target_client_id: fixed(r.get_target_client_id(), "delete target")?,
        })
    }
}

/// 0x0112 DmReaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmReaction {
    pub target_client_id: ClientId,
    pub emoji: String,
    /// `false` removes the reaction.
    pub add: bool,
}

impl DmReaction {
    pub(super) fn write(&self, mut b: dm_reaction::Builder<'_>) {
        b.set_target_client_id(&self.target_client_id);
        b.set_emoji(&self.emoji);
        b.set_add(self.add);
    }

    pub(super) fn read(r: dm_reaction::Reader<'_>) -> Result<Self, CodecError> {
        let emoji = text(r.get_emoji())?;
        // One emoji: a name's rule (not blank, no control or hidden
        // format character) within a reaction's length.
        key_format::name(&emoji, MAX_REACTION_LEN)
            .map_err(|e| CodecError::Deserialization(format!("reaction: {e}")))?;
        Ok(Self {
            target_client_id: fixed(r.get_target_client_id(), "reaction target")?,
            emoji,
            add: r.get_add(),
        })
    }
}

/// 0x0400 ReadUpTo: we have read the peer's messages through `stream_seq`
/// on our DM stream with them (plan E2.5, r10). It carries no timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadUpTo {
    pub stream_seq: u64,
}

impl ReadUpTo {
    pub(super) fn write(&self, mut b: read_up_to::Builder<'_>) {
        b.set_stream_seq(self.stream_seq);
    }

    pub(super) fn read(r: read_up_to::Reader<'_>) -> Self {
        Self {
            stream_seq: r.get_stream_seq(),
        }
    }
}

/// 0x0401 Typing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Typing {
    pub typing: bool,
}

impl Typing {
    pub(super) fn write(&self, mut b: typing::Builder<'_>) {
        b.set_typing(self.typing);
    }

    pub(super) fn read(r: typing::Reader<'_>) -> Self {
        Self {
            typing: r.get_typing(),
        }
    }
}
