//! The body of each kind: the `payload` of a [`super::DmContent`] or
//! [`super::SealedContent`].
//!
//! Kinds §6 marks "moved unchanged" (relay, status, push relay, calls and
//! DM video) keep the pre-E2.3 1:1 payload's fields, names and types
//! one for one (`String` → `Text`, `Vec<u8>` → `Data`, `Option<String>` →
//! `Text`, empty when absent), so the cutover maps each variant without
//! a semantic change. The friendship and content kinds are E2.3's own
//! definitions. Unfriended, UnfriendedAck and
//! SessionResetAccept have an empty body. The group-DM kinds (0x0120-0x0124)
//! are E2.6's sender-key bodies; the lobby kinds (0x0127-0x012B) are E7.2
//! G6's. The creator-set kinds (0x0190-0x0194) are
//! E3.2 S7s.3's.

mod calls;
mod content;
mod creator;
mod friend;
mod game;
mod group;
mod lobby;
mod relay;

pub use calls::{
    CallAccept, CallClosed, CallInvite, CallMediaState, CallReaction, CallRinging, DmVideoFragment,
    GroupCallMember, GroupCallOffer, GroupCallParticipantLeft,
};
#[cfg(test)]
pub(crate) use content::timer_sample;
pub use content::{
    DisappearingTimer, DmAck, DmDelete, DmEdit, DmReaction, DmText, ReadUpTo, Typing,
    MAX_DM_ACK_SEQS,
};
pub use creator::{CreatorAccept, CreatorDecline, CreatorInvite, CreatorJoin, CreatorWithdrawn};
pub use friend::{
    FriendAccept, FriendRequest, FriendsKey, IdentityRotated, PreKeyRejected, ProfileKeyRotated,
    RequestAuth, RetryRequest, SessionResetDecline, SessionResetRequest, StreamRecord,
};
#[cfg(test)]
pub(crate) use game::{
    bio_sample as profile_bio_sample, invite_sample as game_invite_sample,
    sample as game_totals_sample,
};
pub use game::{GameInvite, GameTotal, GameTotals, ProfileBio, MAX_GAME_TOTALS};
pub use group::{
    GroupContent, GroupDmAccept, GroupDmDecline, GroupDmInvite, GroupDmLeave, GroupDmRename,
    GroupDmRosterGrow, GroupId,
};
pub use lobby::{LobbyId, LobbyLaunch, LobbyOpen, LobbyRef};
pub use relay::{
    RegisterPushRelay, RelayEnvelope, RelayOffer, RelayOfferAck, RelayWithdraw, StatusRequest,
    StatusResponse, UnregisterPushRelay, WakeNotify,
};

use super::{decode_root, encode_root, DmTypeId};
use crate::capnp_codec::CodecError;
use crate::direct_capnp as schema;

/// Adapt an infallible scalar or text writer to the root encoder.
macro_rules! write_direct_body {
    (checked, $body:expr, $builder:expr) => {
        $body.write($builder)
    };
    (plain, $body:expr, $builder:expr) => {{
        $body.write($builder);
        Ok(())
    }};
}

/// `bodies` read fallibly (text, data, fixed-size fields); `scalar` bodies
/// hold only scalars, so reading them cannot fail; `empty` kinds carry no
/// bytes.
macro_rules! direct_bodies {
    (
        bodies { $( $kind:ident($ty:ident) = $schema:ident($writer:ident), )* }
        scalar { $( $skind:ident($sty:ident) = $sschema:ident($swriter:ident), )* }
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
                        pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
                match self {
                    $( Self::$kind(body) => {
                        encode_root::<schema::$schema::Owned>(|b| write_direct_body!($writer, body, b))
                    } )*
                    $( Self::$skind(body) => {
                        encode_root::<schema::$sschema::Owned>(|b| write_direct_body!($swriter, body, b))
                    } )*
                    $( Self::$empty => Ok(Vec::new()), )*
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
        FriendRequest(FriendRequest) = friend_request(plain),
        RelayEnvelope(RelayEnvelope) = relay_envelope(plain),
        PreKeyRejected(PreKeyRejected) = pre_key_rejected(plain),
        RetryRequest(RetryRequest) = retry_request(plain),
        SessionResetRequest(SessionResetRequest) = session_reset_request(plain),
        SessionResetDecline(SessionResetDecline) = session_reset_decline(plain),
        DmAck(DmAck) = dm_ack(checked),
        DmText(DmText) = dm_text(plain),
        FriendAccept(FriendAccept) = friend_accept(plain),
        ProfileKeyRotated(ProfileKeyRotated) = profile_key_rotated(plain),
        FriendsKey(FriendsKey) = friends_key(plain),
        StreamRecord(StreamRecord) = stream_record(plain),
        IdentityRotated(IdentityRotated) = identity_rotated(plain),
        DmEdit(DmEdit) = dm_edit(plain),
        DmDelete(DmDelete) = dm_delete(plain),
        DmReaction(DmReaction) = dm_reaction(plain),
        GroupDmInvite(GroupDmInvite) = group_dm_invite(checked),
        GroupDmAccept(GroupDmAccept) = group_dm_accept(plain),
        GroupDmDecline(GroupDmDecline) = group_dm_decline(plain),
        GroupDmLeave(GroupDmLeave) = group_dm_leave(plain),
        GroupContent(GroupContent) = group_content(plain),
        GroupDmRename(GroupDmRename) = group_dm_rename(plain),
        GroupDmRosterGrow(GroupDmRosterGrow) = group_dm_roster_grow(checked),
        LobbyOpen(LobbyOpen) = lobby_open(plain),
        LobbyJoin(LobbyRef) = lobby_ref(plain),
        LobbyLeave(LobbyRef) = lobby_ref(plain),
        LobbyLaunch(LobbyLaunch) = lobby_launch(plain),
        LobbyClose(LobbyRef) = lobby_ref(plain),
        RelayOffer(RelayOffer) = relay_offer(plain),
        RelayWithdraw(RelayWithdraw) = relay_withdraw(plain),
        RelayOfferAck(RelayOfferAck) = relay_offer_ack(plain),
        StatusRequest(StatusRequest) = status_request(plain),
        StatusResponse(StatusResponse) = status_response(plain),
        DisappearingTimer(DisappearingTimer) = disappearing_timer(plain),
        GameInvite(GameInvite) = game_invite(plain),
        ProfileBio(ProfileBio) = profile_bio(plain),
        GameTotals(GameTotals) = game_totals(checked),
        CreatorInvite(CreatorInvite) = creator_invite(plain),
        CreatorAccept(CreatorAccept) = creator_accept(plain),
        CreatorDecline(CreatorDecline) = creator_decline(plain),
        CreatorWithdrawn(CreatorWithdrawn) = creator_withdrawn(plain),
        CreatorJoin(CreatorJoin) = creator_join(plain),
        CallInvite(CallInvite) = call_invite(checked),
        CallRinging(CallRinging) = call_ringing(plain),
        CallAccept(CallAccept) = call_accept(checked),
        CallDecline(CallClosed) = call_closed(plain),
        CallEnd(CallClosed) = call_closed(plain),
        CallMediaState(CallMediaState) = call_media_state(plain),
        CallReaction(CallReaction) = call_reaction(plain),
        GroupCallOffer(GroupCallOffer) = group_call_offer(checked),
        GroupCallAccept(GroupCallMember) = group_call_member(plain),
        GroupCallDecline(CallClosed) = call_closed(plain),
        GroupCallParticipantJoined(GroupCallMember) = group_call_member(plain),
        GroupCallParticipantLeft(GroupCallParticipantLeft) = group_call_participant_left(plain),
        DmVideoFragment(DmVideoFragment) = dm_video_fragment(plain),
        RegisterPushRelay(RegisterPushRelay) = register_push_relay(checked),
        UnregisterPushRelay(UnregisterPushRelay) = unregister_push_relay(plain),
    }
    scalar {
        WakeNotify(WakeNotify) = wake_notify(plain),
        ReadUpTo(ReadUpTo) = read_up_to(plain),
        Typing(Typing) = typing(plain),
    }
    empty {
        SessionResetAccept,
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
