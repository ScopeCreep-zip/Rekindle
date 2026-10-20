//! Friendship and session bodies.

use crate::capnp_codec::direct::{data, fixed, opt_text, text};
use crate::capnp_codec::CodecError;
use crate::direct_capnp::{
    friend_accept, friend_request, identity_rotated, profile_key_rotated, session_reset_decline,
    session_reset_request,
};

/// 0x0002 FriendRequest (sealed, by `app_call`). It carries no bundle:
/// the accepter reads our prekey record (plan E2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendRequest {
    pub display_name: String,
    pub message: String,
    pub profile_dht_key: String,
    pub route_blob: Vec<u8>,
    pub mailbox_dht_key: String,
    /// The invite this request answers.
    pub invite_id: Option<String>,
}

impl FriendRequest {
    pub(super) fn write(&self, mut b: friend_request::Builder<'_>) {
        b.set_display_name(&self.display_name);
        b.set_message(&self.message);
        b.set_profile_dht_key(&self.profile_dht_key);
        b.set_route_blob(&self.route_blob);
        b.set_mailbox_dht_key(&self.mailbox_dht_key);
        b.set_invite_id(self.invite_id.as_deref().unwrap_or_default());
    }

    pub(super) fn read(r: friend_request::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            display_name: text(r.get_display_name())?,
            message: text(r.get_message())?,
            profile_dht_key: text(r.get_profile_dht_key())?,
            route_blob: data(r.get_route_blob())?,
            mailbox_dht_key: text(r.get_mailbox_dht_key())?,
            invite_id: opt_text(r.get_invite_id())?,
        })
    }
}

/// 0x0101 FriendAccept, the first payload of a PreKeyInit. The session
/// the init opens is the acceptance, and the init carries what today's
/// `FriendAccept` carried for `respond_to_session`; the requester never
/// initiates from our bundle, so none is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendAccept {
    pub profile_dht_key: String,
    pub route_blob: Vec<u8>,
    pub mailbox_dht_key: String,
}

impl FriendAccept {
    pub(super) fn write(&self, mut b: friend_accept::Builder<'_>) {
        b.set_profile_dht_key(&self.profile_dht_key);
        b.set_route_blob(&self.route_blob);
        b.set_mailbox_dht_key(&self.mailbox_dht_key);
    }

    pub(super) fn read(r: friend_accept::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            profile_dht_key: text(r.get_profile_dht_key())?,
            route_blob: data(r.get_route_blob())?,
            mailbox_dht_key: text(r.get_mailbox_dht_key())?,
        })
    }
}

/// 0x0007 SessionResetRequest (sealed). The requester is the frame's
/// sender (its pinned identity); the accepter initiates from the
/// requester's prekey record (plan E2.4), so no bundle travels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionResetRequest {
    /// Why the requester asks, shown in the accepter's review.
    pub reason: String,
}

impl SessionResetRequest {
    pub(super) fn write(&self, mut b: session_reset_request::Builder<'_>) {
        b.set_reason(&self.reason);
    }

    pub(super) fn read(r: session_reset_request::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            reason: text(r.get_reason())?,
        })
    }
}

/// 0x0009 SessionResetDecline (sealed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionResetDecline {
    pub reason: String,
}

impl SessionResetDecline {
    pub(super) fn write(&self, mut b: session_reset_decline::Builder<'_>) {
        b.set_reason(&self.reason);
    }

    pub(super) fn read(r: session_reset_decline::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            reason: text(r.get_reason())?,
        })
    }
}

/// 0x0105 ProfileKeyRotated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileKeyRotated {
    pub new_profile_dht_key: String,
}

impl ProfileKeyRotated {
    pub(super) fn write(&self, mut b: profile_key_rotated::Builder<'_>) {
        b.set_new_profile_dht_key(&self.new_profile_dht_key);
    }

    pub(super) fn read(r: profile_key_rotated::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            new_profile_dht_key: text(r.get_new_profile_dht_key())?,
        })
    }
}

/// 0x0108 IdentityRotated, over the old session. Both signatures are over
/// `IDENTITY_ROTATED ‖ old ‖ new ‖ recipient`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityRotated {
    pub new_identity: [u8; 32],
    /// By the old identity.
    pub sig_old: [u8; 64],
    /// By the new identity.
    pub sig_new: [u8; 64],
}

impl IdentityRotated {
    pub(super) fn write(&self, mut b: identity_rotated::Builder<'_>) {
        b.set_new_identity(&self.new_identity);
        b.set_sig_old(&self.sig_old);
        b.set_sig_new(&self.sig_new);
    }

    pub(super) fn read(r: identity_rotated::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            new_identity: fixed(r.get_new_identity(), "new identity")?,
            sig_old: fixed(r.get_sig_old(), "old identity signature")?,
            sig_new: fixed(r.get_sig_new(), "new identity signature")?,
        })
    }
}
