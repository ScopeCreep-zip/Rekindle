//! The body of each kind: the `payload` of a [`super::DmContent`] or
//! [`super::SealedContent`].
//!
//! Kinds §6 marks "moved unchanged" (relay, status, push relay, calls and
//! DM video) keep the pre-E2.3 1:1 payload's fields, names and types
//! one for one (`String` → `Text`, `Vec<u8>` → `Data`, `Option<String>` →
//! `Text`, empty when absent), so the cutover maps each variant without
//! a semantic change. The friendship and content kinds are E2.3's own
//! definitions. FriendReject, Unfriended, UnfriendedAck and
//! SessionResetAccept have an empty body. GroupDmInvite and GroupDmLeave
//! carry the SMPL group path's fields unchanged until E2.6 replaces them.
//! RetryRequest (E2.5), GroupDmAccept and GroupDmDecline (E2.6) are
//! allocated here; their steps define their bodies, so
//! [`DirectBody::decode`] refuses them until then.

mod calls;
mod content;
mod friend;
mod group;
mod relay;

pub use calls::{
    CallAccept, CallClosed, CallInvite, CallMediaState, CallReaction, CallRinging, DmVideoFragment,
    GroupCallMember, GroupCallOffer, GroupCallParticipantLeft,
};
pub use content::{DmDelete, DmEdit, DmReaction, DmText, Typing};
pub use friend::{
    FriendAccept, FriendRequest, IdentityRotated, ProfileKeyRotated, SessionResetDecline,
    SessionResetRequest,
};
pub use group::{GroupDmInvite, GroupDmLeave};
pub use relay::{
    RegisterPushRelay, RelayEnvelope, RelayOffer, RelayOfferAck, RelayWithdraw, StatusRequest,
    StatusResponse, UnregisterPushRelay, WakeNotify,
};

use super::{decode_root, encode_root, DmTypeId};
use crate::capnp_codec::CodecError;
use crate::direct_capnp as schema;

/// `bodies` read fallibly (text, data, fixed-size fields); `scalar` bodies
/// hold only scalars, so reading them cannot fail; `empty` kinds carry no
/// bytes.
macro_rules! direct_bodies {
    (
        bodies { $( $kind:ident($ty:ident) = $schema:ident, )* }
        scalar { $( $skind:ident($sty:ident) = $sschema:ident, )* }
        empty { $( $empty:ident, )* }
    ) => {
        /// A decoded body, by kind.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum DirectBody {
            $( $kind($ty), )*
            $( $skind($sty), )*
            $( $empty, )*
        }

        impl DirectBody {
            /// The kind this body is the body of.
            #[must_use]
            pub fn type_id(&self) -> DmTypeId {
                match self {
                    $( Self::$kind(_) => DmTypeId::$kind, )*
                    $( Self::$skind(_) => DmTypeId::$skind, )*
                    $( Self::$empty => DmTypeId::$empty, )*
                }
            }

            /// The body's bytes (packed Cap'n Proto; empty for the kinds
            /// without a body).
            #[must_use]
            pub fn encode(&self) -> Vec<u8> {
                match self {
                    $( Self::$kind(body) => {
                        encode_root::<schema::$schema::Owned>(|b| body.write(b))
                    } )*
                    $( Self::$skind(body) => {
                        encode_root::<schema::$sschema::Owned>(|b| body.write(b))
                    } )*
                    $( Self::$empty => Vec::new(), )*
                }
            }

            /// Decode the body of `type_id`.
            ///
            /// # Errors
            /// The bytes are not that kind's body, or the kind has no body
            /// (PreKeyInit, which is a frame) or none yet (`UnknownVariant`).
            pub fn decode(type_id: DmTypeId, bytes: &[u8]) -> Result<Self, CodecError> {
                match type_id {
                    $( DmTypeId::$kind => {
                        decode_root::<schema::$schema::Owned, _>(bytes, $ty::read).map(Self::$kind)
                    } )*
                    $( DmTypeId::$skind => {
                        decode_root::<schema::$sschema::Owned, _>(bytes, |r| Ok($sty::read(r)))
                            .map(Self::$skind)
                    } )*
                    $( DmTypeId::$empty => empty(type_id, bytes).map(|()| Self::$empty), )*
                    other => Err(CodecError::UnknownVariant(format!(
                        "{other:?} has no DirectBody"
                    ))),
                }
            }
        }
    };
}

direct_bodies! {
    bodies {
        FriendRequest(FriendRequest) = friend_request,
        RelayEnvelope(RelayEnvelope) = relay_envelope,
        SessionResetRequest(SessionResetRequest) = session_reset_request,
        SessionResetDecline(SessionResetDecline) = session_reset_decline,
        DmText(DmText) = dm_text,
        FriendAccept(FriendAccept) = friend_accept,
        ProfileKeyRotated(ProfileKeyRotated) = profile_key_rotated,
        IdentityRotated(IdentityRotated) = identity_rotated,
        DmEdit(DmEdit) = dm_edit,
        DmDelete(DmDelete) = dm_delete,
        DmReaction(DmReaction) = dm_reaction,
        GroupDmInvite(GroupDmInvite) = group_dm_invite,
        GroupDmLeave(GroupDmLeave) = group_dm_leave,
        RelayOffer(RelayOffer) = relay_offer,
        RelayWithdraw(RelayWithdraw) = relay_withdraw,
        RelayOfferAck(RelayOfferAck) = relay_offer_ack,
        StatusRequest(StatusRequest) = status_request,
        StatusResponse(StatusResponse) = status_response,
        CallInvite(CallInvite) = call_invite,
        CallRinging(CallRinging) = call_ringing,
        CallAccept(CallAccept) = call_accept,
        CallDecline(CallClosed) = call_closed,
        CallEnd(CallClosed) = call_closed,
        CallMediaState(CallMediaState) = call_media_state,
        CallReaction(CallReaction) = call_reaction,
        GroupCallOffer(GroupCallOffer) = group_call_offer,
        GroupCallAccept(GroupCallMember) = group_call_member,
        GroupCallDecline(CallClosed) = call_closed,
        GroupCallParticipantJoined(GroupCallMember) = group_call_member,
        GroupCallParticipantLeft(GroupCallParticipantLeft) = group_call_participant_left,
        DmVideoFragment(DmVideoFragment) = dm_video_fragment,
        RegisterPushRelay(RegisterPushRelay) = register_push_relay,
        UnregisterPushRelay(UnregisterPushRelay) = unregister_push_relay,
    }
    scalar {
        WakeNotify(WakeNotify) = wake_notify,
        Typing(Typing) = typing,
    }
    empty {
        SessionResetAccept,
        FriendReject,
        Unfriended,
        UnfriendedAck,
    }
}

/// An empty body.
fn empty(type_id: DmTypeId, bytes: &[u8]) -> Result<(), CodecError> {
    if bytes.is_empty() {
        Ok(())
    } else {
        Err(CodecError::Deserialization(format!(
            "{type_id:?} has an empty body, got {} bytes",
            bytes.len()
        )))
    }
}
