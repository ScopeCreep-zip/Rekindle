//! The creator-set round's DM half (plan E3.2 S7s.3, decisions (16),
//! (17), (20)): the creator names friends co-creators of a reserved
//! community (`CreatorInvite`, 0x0190); each answers with its pseudonym for
//! the new anchor, signed (`CreatorAccept`, 0x0191), or declines
//! (`CreatorDecline`, 0x0192); the creator withdraws an invite it removed
//! or abandoned (`CreatorWithdrawn`, 0x0193) and, once the genesis lists a
//! co-creator, sends it the invite link it joins with (`CreatorJoin`,
//! 0x0194). Every kind is ratchet-sealed to one accepted friend and is
//! taken only from one: anyone else's is refused, never applied. What a
//! frame means is the Host's ([`FriendHost::on_creator_round`]).

use rekindle_codec::capnp_codec::direct::{
    CreatorAccept, CreatorDecline, CreatorInvite, CreatorJoin, CreatorWithdrawn, DirectBody,
};
use rekindle_types::friend::FriendshipState;

use super::{state, FriendError, FriendHost};
use crate::operations::dm;

/// A creator-set frame from a friend, as the Host applies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreatorFrame {
    /// The friend names us a co-creator of `community_name` at `anchor`.
    Invite {
        anchor: String,
        community_name: String,
    },
    /// The friend accepts our invite: its pseudonym for `anchor`, signed.
    Accept {
        anchor: String,
        pseudonym: [u8; 32],
        signature: [u8; 64],
    },
    /// The friend declines our invite.
    Decline { anchor: String },
    /// The friend removed us from, or abandoned, its creation of `anchor`.
    Withdrawn { anchor: String },
    /// The genesis of `anchor` lists us: join it with `invite`.
    Join { anchor: String, invite: String },
}

impl CreatorFrame {
    /// The frame's body.
    #[must_use]
    pub fn body(&self) -> DirectBody {
        match self.clone() {
            Self::Invite {
                anchor,
                community_name,
            } => DirectBody::CreatorInvite(CreatorInvite {
                anchor,
                community_name,
            }),
            Self::Accept {
                anchor,
                pseudonym,
                signature,
            } => DirectBody::CreatorAccept(CreatorAccept {
                anchor,
                pseudonym,
                signature,
            }),
            Self::Decline { anchor } => DirectBody::CreatorDecline(CreatorDecline { anchor }),
            Self::Withdrawn { anchor } => DirectBody::CreatorWithdrawn(CreatorWithdrawn { anchor }),
            Self::Join { anchor, invite } => {
                DirectBody::CreatorJoin(CreatorJoin { anchor, invite })
            }
        }
    }

    /// The frame a creator-set body is; `None` for any other kind.
    #[must_use]
    pub fn of(body: &DirectBody) -> Option<Self> {
        Some(match body.clone() {
            DirectBody::CreatorInvite(b) => Self::Invite {
                anchor: b.anchor,
                community_name: b.community_name,
            },
            DirectBody::CreatorAccept(b) => Self::Accept {
                anchor: b.anchor,
                pseudonym: b.pseudonym,
                signature: b.signature,
            },
            DirectBody::CreatorDecline(b) => Self::Decline { anchor: b.anchor },
            DirectBody::CreatorWithdrawn(b) => Self::Withdrawn { anchor: b.anchor },
            DirectBody::CreatorJoin(b) => Self::Join {
                anchor: b.anchor,
                invite: b.invite,
            },
            _ => return None,
        })
    }
}

/// Whether `peer` is an accepted friend, the only peer a creator-set
/// frame goes to or is taken from.
#[must_use]
pub fn is_accepted_friend(host: &dyn FriendHost, peer: &str) -> bool {
    state::friend(host, peer).map(|f| f.friendship_state) == Some(FriendshipState::Accepted)
}

/// Send `frame` to `peer`, an accepted friend. The outbox id of the
/// committed frame.
///
/// # Errors
/// `peer` is not an accepted friend, or the frame could not be committed.
pub async fn send(
    host: &dyn FriendHost,
    peer: &str,
    frame: &CreatorFrame,
) -> Result<String, FriendError> {
    if !is_accepted_friend(host, peer) {
        return Err(FriendError::NotFound(format!(
            "{peer} is not an accepted friend"
        )));
    }
    dm::handshake::enqueue(host, peer, &frame.body()).await
}

/// A creator-set frame from `sender`: applied by the Host only from an
/// accepted friend; anyone else's is refused (logged, never applied).
/// Whether it was handled.
pub(crate) async fn on_frame(host: &dyn FriendHost, sender: &str, body: &DirectBody) -> bool {
    let Some(frame) = CreatorFrame::of(body) else {
        return true;
    };
    if !is_accepted_friend(host, sender) {
        tracing::warn!(from = %sender, kind = ?body.type_id(), "creator-set frame from a non-friend refused");
        return true;
    }
    host.on_creator_round(sender, frame).await
}
