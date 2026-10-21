//! Friendship and session bodies.

use rekindle_types::key_format::{self, FRIEND_REQUEST_TOKEN_LEN, MAX_INVITE_ID_LEN, MAX_NOTE_LEN};

use crate::capnp_codec::direct::{
    bounded_text, data, display_name, fixed, opt_record_key, opt_text, record_key, PqPrekey,
};
use crate::capnp_codec::CodecError;
use crate::direct_capnp::{
    friend_accept, friend_request, friends_key, identity_rotated, pre_key_rejected,
    profile_key_rotated, retry_request, session_reset_decline, session_reset_request,
};

/// What admits a friend request (plan E2.5, D17): exactly one of the
/// target's friend-code token or one of its invites. The two paths stay
/// apart on the wire and in the receiver's admission.
#[derive(Clone, PartialEq, Eq)]
pub enum RequestAuth {
    /// The request-token our friend code carried.
    Token(String),
    /// The invite this request answers (its id; the invite capability).
    Invite(String),
}

/// The token is never printed.
impl std::fmt::Debug for RequestAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Token(_) => f.write_str("Token(..)"),
            Self::Invite(id) => f.debug_tuple("Invite").field(id).finish(),
        }
    }
}

/// 0x0002 FriendRequest (sealed, by `app_call`). It carries no bundle:
/// the accepter reads our prekey record (plan E2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendRequest {
    pub display_name: String,
    pub message: String,
    pub profile_dht_key: String,
    pub route_blob: Vec<u8>,
    pub mailbox_dht_key: String,
    /// The token or the invite that admits it.
    pub auth: RequestAuth,
}

impl FriendRequest {
    pub(super) fn write(&self, mut b: friend_request::Builder<'_>) {
        b.set_display_name(&self.display_name);
        b.set_message(&self.message);
        b.set_profile_dht_key(&self.profile_dht_key);
        b.set_route_blob(&self.route_blob);
        b.set_mailbox_dht_key(&self.mailbox_dht_key);
        match &self.auth {
            RequestAuth::Token(token) => b.set_request_token(token),
            RequestAuth::Invite(id) => b.set_invite_id(id),
        }
    }

    pub(super) fn read(r: friend_request::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            display_name: display_name(r.get_display_name())?,
            message: bounded_text(r.get_message(), MAX_NOTE_LEN, "request message")?,
            // A request names the profile the accepter opens the session
            // from (plan E2.4); the sender never sends one without it.
            profile_dht_key: record_key(r.get_profile_dht_key(), "profile key")?,
            route_blob: data(r.get_route_blob())?,
            mailbox_dht_key: opt_record_key(r.get_mailbox_dht_key(), "mailbox key")?,
            auth: request_auth(r)?,
        })
    }
}

/// The one admission a request names: a token of the friend-code shape,
/// or an invite id; both or neither is refused.
fn request_auth(r: friend_request::Reader<'_>) -> Result<RequestAuth, CodecError> {
    let invite = opt_text(r.get_invite_id(), MAX_INVITE_ID_LEN, "invite id")?;
    let token = opt_text(
        r.get_request_token(),
        FRIEND_REQUEST_TOKEN_LEN,
        "request token",
    )?;
    match (token, invite) {
        (Some(token), None) => {
            key_format::friend_request_token(&token)
                .map_err(|e| CodecError::Deserialization(format!("request token: {e}")))?;
            Ok(RequestAuth::Token(token))
        }
        (None, Some(id)) => Ok(RequestAuth::Invite(id)),
        (Some(_), Some(_)) => Err(CodecError::Deserialization(
            "friend request names both a token and an invite".into(),
        )),
        (None, None) => Err(CodecError::Deserialization(
            "friend request names neither a token nor an invite".into(),
        )),
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
            profile_dht_key: opt_record_key(r.get_profile_dht_key(), "profile key")?,
            route_blob: data(r.get_route_blob())?,
            mailbox_dht_key: opt_record_key(r.get_mailbox_dht_key(), "mailbox key")?,
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
            reason: bounded_text(r.get_reason(), MAX_NOTE_LEN, "reason")?,
        })
    }
}

/// 0x0005 PreKeyRejected (sealed): the prekeys a PreKeyInit named, which
/// the responder no longer holds, and the hash of its base key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreKeyRejected {
    pub signed_prekey_id: u32,
    pub one_time_prekey_id: Option<u32>,
    pub pq_prekey: PqPrekey,
    pub base_key_hash: [u8; 32],
}

impl PreKeyRejected {
    pub(super) fn write(&self, mut b: pre_key_rejected::Builder<'_>) {
        b.set_signed_prekey_id(self.signed_prekey_id);
        match self.one_time_prekey_id {
            Some(id) => b.reborrow().init_one_time_prekey().set_id(id),
            None => b.reborrow().init_one_time_prekey().set_none(()),
        }
        match self.pq_prekey {
            PqPrekey::OneTime(id) => b.reborrow().init_pq_prekey().set_one_time(id),
            PqPrekey::LastResort(id) => b.reborrow().init_pq_prekey().set_last_resort(id),
        }
        b.set_base_key_hash(&self.base_key_hash);
    }

    pub(super) fn read(r: pre_key_rejected::Reader<'_>) -> Result<Self, CodecError> {
        use pre_key_rejected::{one_time_prekey, pq_prekey};
        let one_time_prekey_id = match r
            .get_one_time_prekey()
            .which()
            .map_err(|e| CodecError::Deserialization(format!("one-time prekey: {e}")))?
        {
            one_time_prekey::None(()) => None,
            one_time_prekey::Id(id) => Some(id),
        };
        let pq_prekey = match r
            .get_pq_prekey()
            .which()
            .map_err(|e| CodecError::Deserialization(format!("pq prekey: {e}")))?
        {
            pq_prekey::OneTime(id) => PqPrekey::OneTime(id),
            pq_prekey::LastResort(id) => PqPrekey::LastResort(id),
        };
        Ok(Self {
            signed_prekey_id: r.get_signed_prekey_id(),
            one_time_prekey_id,
            pq_prekey,
            base_key_hash: fixed(r.get_base_key_hash(), "base key hash")?,
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
            reason: bounded_text(r.get_reason(), MAX_NOTE_LEN, "reason")?,
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
            new_profile_dht_key: record_key(r.get_new_profile_dht_key(), "profile key")?,
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

/// 0x0106 FriendsKey, inside the ratchet: the key our profile's STATUS and
/// GAME_INFO subkeys are sealed under (plan E2.5).
#[derive(Clone, PartialEq, Eq)]
pub struct FriendsKey {
    pub epoch: u32,
    pub key: [u8; 32],
}

/// The key is never printed.
impl std::fmt::Debug for FriendsKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FriendsKey")
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

impl FriendsKey {
    pub(super) fn write(&self, mut b: friends_key::Builder<'_>) {
        b.set_epoch(self.epoch);
        b.set_key(&self.key);
    }

    pub(super) fn read(r: friends_key::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            epoch: r.get_epoch(),
            key: fixed(r.get_key(), "friends key")?,
        })
    }
}

/// 0x0006 RetryRequest, inside the ratchet: a message of the sender's we
/// could not open, named by its header id and the ratchet key we observed
/// on it (plan E2.5, D24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryRequest {
    pub msg_id: [u8; 32],
    pub observed_ratchet_key: [u8; 32],
    /// Milliseconds.
    pub ts: u64,
}

impl RetryRequest {
    pub(super) fn write(&self, mut b: retry_request::Builder<'_>) {
        b.set_msg_id(&self.msg_id);
        b.set_observed_ratchet_key(&self.observed_ratchet_key);
        b.set_ts(self.ts);
    }

    pub(super) fn read(r: retry_request::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            msg_id: fixed(r.get_msg_id(), "retry msg id")?,
            observed_ratchet_key: fixed(r.get_observed_ratchet_key(), "observed ratchet key")?,
            ts: r.get_ts(),
        })
    }
}
