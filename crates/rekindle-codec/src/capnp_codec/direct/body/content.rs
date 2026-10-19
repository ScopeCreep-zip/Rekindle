//! Content bodies: text, its edits, deletes and reactions, and typing.

use crate::capnp_codec::direct::{fixed, text, ClientId};
use crate::capnp_codec::{capnp_err, CodecError};
use crate::direct_capnp::{dm_delete, dm_edit, dm_reaction, dm_text, typing};

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
            body: text(r.get_body())?,
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
            body: text(r.get_body())?,
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
        Ok(Self {
            target_client_id: fixed(r.get_target_client_id(), "reaction target")?,
            emoji: text(r.get_emoji())?,
            add: r.get_add(),
        })
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
