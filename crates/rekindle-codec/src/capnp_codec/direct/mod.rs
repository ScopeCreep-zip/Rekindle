//! The 1:1 wire (schemas/direct.capnp, plan E2.3): the `DmFrame` every
//! 1:1 datagram carries after the `'D'` tag, its plaintexts, the §6
//! typeIds, and the body of each kind.
//!
//! This crate holds the bytes only; sealing (the ratchet, PQXDH, HPKE)
//! and the signature over a sealed frame are the caller's. A decoded
//! frame is shape-checked: the body is the one its typeId's [`Carrier`]
//! names, every fixed-size field has its length, and a PreKeyInit's first
//! payload is a kind a PreKeyInit may carry.

mod body;
mod frame;
#[cfg(test)]
mod tests;
mod type_id;

pub use body::{
    CallAccept, CallClosed, CallInvite, CallMediaState, CallReaction, CallRinging, DirectBody,
    DmDelete, DmEdit, DmReaction, DmText, DmVideoFragment, FriendAccept, FriendRequest,
    GroupCallMember, GroupCallOffer, GroupCallParticipantLeft, GroupDmInvite, GroupDmLeave,
    IdentityRotated, ProfileKeyRotated, RegisterPushRelay, RelayEnvelope, RelayOffer,
    RelayOfferAck, RelayWithdraw, SessionResetDecline, SessionResetRequest, StatusRequest,
    StatusResponse, Typing, UnregisterPushRelay, WakeNotify,
};
pub use frame::{
    ContentHint, DmContent, DmFrame, FrameBody, PqPrekey, PreKeyInitBody, RatchetBody,
    RatchetHeader, SealedBody, SealedContent,
};
pub use type_id::{Carrier, DmTypeId};

use super::{capnp_err, pack, unpack, CodecError};

/// A message's client id: the UUID v7 its author mints at send.
pub type ClientId = [u8; 16];

/// Packed bytes of a root struct built by `write`.
fn encode_root<O: capnp::traits::Owned>(write: impl FnOnce(O::Builder<'_>)) -> Vec<u8> {
    let mut message = capnp::message::Builder::new_default();
    write(message.init_root::<O::Builder<'_>>());
    pack(&message)
}

/// Read a root struct from packed bytes with `read`.
fn decode_root<O: capnp::traits::Owned, R>(
    bytes: &[u8],
    read: impl FnOnce(O::Reader<'_>) -> Result<R, CodecError>,
) -> Result<R, CodecError> {
    let message = unpack(bytes)?;
    let root = message
        .get_root::<O::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    read(root)
}

/// A `Data` field of exactly `N` bytes.
fn fixed<const N: usize>(
    field: Result<&[u8], capnp::Error>,
    what: &str,
) -> Result<[u8; N], CodecError> {
    let bytes = field.map_err(|e| capnp_err(&e))?;
    bytes.try_into().map_err(|_| {
        CodecError::Deserialization(format!("{what}: {} bytes, want {N}", bytes.len()))
    })
}

/// A `Data` field, owned.
fn data(field: Result<&[u8], capnp::Error>) -> Result<Vec<u8>, CodecError> {
    Ok(field.map_err(|e| capnp_err(&e))?.to_vec())
}

/// A `Text` field, owned.
fn text(field: Result<capnp::text::Reader<'_>, capnp::Error>) -> Result<String, CodecError> {
    super::text_to_string(field.map_err(|e| capnp_err(&e))?)
}

/// A `Text` field where empty means absent.
fn opt_text(
    field: Result<capnp::text::Reader<'_>, capnp::Error>,
) -> Result<Option<String>, CodecError> {
    let s = text(field)?;
    Ok((!s.is_empty()).then_some(s))
}

/// A `List(Text)` field, owned.
fn text_list(
    field: Result<capnp::text_list::Reader<'_>, capnp::Error>,
) -> Result<Vec<String>, CodecError> {
    field.map_err(|e| capnp_err(&e))?.iter().map(text).collect()
}

/// Write `items` into a `List(Text)` field.
fn set_text_list(mut list: capnp::text_list::Builder<'_>, items: &[String]) {
    for (i, item) in items.iter().enumerate() {
        list.set(
            u32::try_from(i).expect("a list built here fits a u32 index"),
            item.as_str(),
        );
    }
}

/// The length of a list field for `items`.
fn list_len<T>(items: &[T]) -> u32 {
    u32::try_from(items.len()).expect("a 1:1 datagram's list fits a u32 length")
}
