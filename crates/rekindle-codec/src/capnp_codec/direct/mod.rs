//! The 1:1 wire (schemas/direct.capnp, plan E2.3): the `DmFrame` every
//! 1:1 datagram carries after the `'D'` tag, its plaintexts, the §6
//! typeIds, and the body of each kind.
//!
//! This crate holds the bytes only; sealing (the ratchet, PQXDH, HPKE)
//! and the signature over a sealed frame are the caller's. A decoded
//! frame is shape-checked: the body is the one its typeId's [`Carrier`]
//! names, every fixed-size field has its length, and a PreKeyInit's first
//! payload is a kind a PreKeyInit may carry.
//!
//! Everything here decodes bytes off the network, so the decoder holds the
//! network-side caps (plan §5 `validate_request` row; audit G6): a frame
//! over Veilid's message cap is refused before it is unpacked, no message
//! unpacks past [`MAX_UNPACKED_WORDS`], and the text a peer supplies — a
//! display name, a message, a reason, a status — is held to the bounds and
//! the character rule the IPC validation holds a user to
//! (`rekindle_types::key_format`, `presence::limits`), keys and record
//! keys to their shapes.

mod body;
mod frame;
#[cfg(test)]
mod tests;
mod type_id;

pub use body::{
    CallAccept, CallClosed, CallInvite, CallMediaState, CallReaction, CallRinging, DirectBody,
    DmDelete, DmEdit, DmReaction, DmText, DmVideoFragment, FriendAccept, FriendRequest, FriendsKey,
    GroupCallMember, GroupCallOffer, GroupCallParticipantLeft, GroupDmInvite, GroupDmLeave,
    IdentityRotated, PreKeyRejected, ProfileKeyRotated, RegisterPushRelay, RelayEnvelope,
    RelayOffer, RelayOfferAck, RelayWithdraw, SessionResetDecline, SessionResetRequest,
    StatusRequest, StatusResponse, Typing, UnregisterPushRelay, WakeNotify,
};
pub use frame::{
    ContentHint, DmContent, DmFrame, FrameBody, PqPrekey, PreKeyInitBody, RatchetBody,
    RatchetHeader, SealedBody, SealedContent,
};
pub use type_id::{Carrier, DmTypeId};

use rekindle_types::key_format;

use super::{capnp_err, pack, unpack_bounded, CodecError};

/// A message's client id: the UUID v7 its author mints at send.
pub type ClientId = [u8; 16];

/// The most words any 1:1 message unpacks to: four times what Veilid's
/// whole message cap could carry unpacked. A legitimate frame unpacks to
/// about its packed size; a zero-word run packs 256 words into two bytes,
/// so without this a 32 KiB datagram could unpack to 32 MiB.
pub const MAX_UNPACKED_WORDS: usize = 4 * rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN / 8;

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
    let message = unpack_bounded(bytes, MAX_UNPACKED_WORDS)?;
    let root = message
        .get_root::<O::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    read(root)
}

/// A `Data` field of exactly `N` bytes.
pub(super) fn fixed<const N: usize>(
    field: Result<&[u8], capnp::Error>,
    what: &str,
) -> Result<[u8; N], CodecError> {
    let bytes = field.map_err(|e| capnp_err(&e))?;
    bytes.try_into().map_err(|_| {
        CodecError::Deserialization(format!("{what}: {} bytes, want {N}", bytes.len()))
    })
}

/// A `Data` field, owned.
pub(super) fn data(field: Result<&[u8], capnp::Error>) -> Result<Vec<u8>, CodecError> {
    Ok(field.map_err(|e| capnp_err(&e))?.to_vec())
}

/// A `Text` field, owned.
fn text(field: Result<capnp::text::Reader<'_>, capnp::Error>) -> Result<String, CodecError> {
    super::text_to_string(field.map_err(|e| capnp_err(&e))?)
}

/// A [`bounded_text`] field where empty means absent.
fn opt_text(
    field: Result<capnp::text::Reader<'_>, capnp::Error>,
    max: usize,
    what: &str,
) -> Result<Option<String>, CodecError> {
    let s = bounded_text(field, max, what)?;
    Ok((!s.is_empty()).then_some(s))
}

/// A field's value refused by its rule.
fn refused(what: &str, e: &key_format::KeyFormatError) -> CodecError {
    CodecError::Deserialization(format!("{what}: {e}"))
}

/// A `Text` field a peer wrote, within `max` characters, holding no
/// control or hidden format character ([`key_format::text`]).
fn bounded_text(
    field: Result<capnp::text::Reader<'_>, capnp::Error>,
    max: usize,
    what: &str,
) -> Result<String, CodecError> {
    let s = text(field)?;
    key_format::text(&s, max).map_err(|e| refused(what, &e))?;
    Ok(s)
}

/// A `Text` field holding a peer's display name
/// ([`rekindle_types::presence::limits::peer_display_name`]).
fn display_name(
    field: Result<capnp::text::Reader<'_>, capnp::Error>,
) -> Result<String, CodecError> {
    let s = text(field)?;
    rekindle_types::presence::limits::peer_display_name(&s)
        .map_err(|e| refused("display name", &e))?;
    Ok(s)
}

/// A `Text` field holding a message body ([`key_format::message_body`]).
fn message_body(
    field: Result<capnp::text::Reader<'_>, capnp::Error>,
) -> Result<String, CodecError> {
    let s = text(field)?;
    key_format::message_body(&s).map_err(|e| refused("message body", &e))?;
    Ok(s)
}

/// A `Text` field holding a DHT record key.
fn record_key(
    field: Result<capnp::text::Reader<'_>, capnp::Error>,
    what: &str,
) -> Result<String, CodecError> {
    let s = text(field)?;
    key_format::record_key(&s).map_err(|e| refused(what, &e))?;
    Ok(s)
}

/// A `Text` field holding a DHT record key, or empty when there is none.
fn opt_record_key(
    field: Result<capnp::text::Reader<'_>, capnp::Error>,
    what: &str,
) -> Result<String, CodecError> {
    let s = text(field)?;
    if !s.is_empty() {
        key_format::record_key(&s).map_err(|e| refused(what, &e))?;
    }
    Ok(s)
}

/// A `Text` field holding an identity key (64 lowercase hex).
fn public_key(
    field: Result<capnp::text::Reader<'_>, capnp::Error>,
    what: &str,
) -> Result<String, CodecError> {
    let s = text(field)?;
    key_format::public_key_hex(&s).map_err(|e| refused(what, &e))?;
    Ok(s)
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
