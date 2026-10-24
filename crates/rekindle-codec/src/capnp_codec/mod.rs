//! Cap'n Proto encode/decode functions bridging Rust domain types to binary wire format.
//!
//! Each sub-module corresponds to a `.capnp` schema file and provides symmetric
//! `encode_*` / `decode_*` pairs. All functions produce packed Cap'n Proto bytes
//! (smaller than unpacked, suitable for DHT storage and Veilid `app_message`).

use crate::error::CodecError;

pub(crate) fn capnp_err(e: &capnp::Error) -> CodecError {
    CodecError::Deserialization(format!("capnp: {e}"))
}

pub(crate) fn not_in_schema(e: capnp::NotInSchema) -> CodecError {
    CodecError::UnknownVariant(format!("capnp enum: {e}"))
}

/// Convert a capnp text reader to an owned String.
pub(crate) fn text_to_string(t: capnp::text::Reader<'_>) -> Result<String, CodecError> {
    t.to_str()
        .map(std::borrow::ToOwned::to_owned)
        .map_err(|e| CodecError::Deserialization(format!("invalid UTF-8 in capnp text: {e}")))
}

/// Serialize a Cap'n Proto builder into packed bytes.
/// A wire type signed with Ed25519 over its domain-prefixed signing
/// bytes: the voice packet and transport feedback share one definition of
/// signing and strict verification.
pub trait SignedWire {
    /// Names the type in verification errors.
    const WHAT: &'static str;
    /// The domain-prefixed bytes the signature covers.
    fn signing_bytes(&self) -> Vec<u8>;
    /// The signer's public key (32 bytes).
    fn signer_key(&self) -> &[u8];
    /// The signature (64 bytes).
    fn signature(&self) -> &[u8];
    /// Store a signature.
    fn set_signature(&mut self, sig: Vec<u8>);

    /// Sign with `key`, whose public half must be [`Self::signer_key`].
    fn sign(&mut self, key: &rekindle_secrets::ed25519_dalek::SigningKey) {
        let sig = ed25519_sign(key, &self.signing_bytes());
        self.set_signature(sig);
    }

    /// Strictly verify the signature against [`Self::signer_key`].
    ///
    /// # Errors
    /// A malformed key or signature, or one that does not verify.
    fn verify(&self) -> Result<(), CodecError> {
        ed25519_verify(
            self.signer_key(),
            self.signature(),
            &self.signing_bytes(),
            Self::WHAT,
        )
    }
}

/// Ed25519-sign `bytes` (a type's domain-prefixed signing bytes).
pub(crate) fn ed25519_sign(
    key: &rekindle_secrets::ed25519_dalek::SigningKey,
    bytes: &[u8],
) -> Vec<u8> {
    use rekindle_secrets::ed25519_dalek::Signer;
    key.sign(bytes).to_bytes().to_vec()
}

/// Strictly verify Ed25519 `sig` by `key` over `bytes`; `what` names the
/// signed type in errors.
pub(crate) fn ed25519_verify(
    key: &[u8],
    sig: &[u8],
    bytes: &[u8],
    what: &str,
) -> Result<(), CodecError> {
    use rekindle_secrets::ed25519_dalek::{Signature, VerifyingKey};
    let key: [u8; 32] = key
        .try_into()
        .map_err(|_| CodecError::Verification(format!("{what} key length")))?;
    let key = VerifyingKey::from_bytes(&key)
        .map_err(|e| CodecError::Verification(format!("{what} key: {e}")))?;
    let sig: [u8; 64] = sig
        .try_into()
        .map_err(|_| CodecError::Verification(format!("{what} signature length")))?;
    key.verify_strict(bytes, &Signature::from_bytes(&sig))
        .map_err(|e| CodecError::Verification(format!("{what} signature: {e}")))
}

pub(crate) fn pack(builder: &capnp::message::Builder<capnp::message::HeapAllocator>) -> Vec<u8> {
    let mut output = Vec::new();
    capnp::serialize_packed::write_message(&mut output, builder).expect("write to Vec never fails");
    output
}

/// Deserialize packed bytes into a Cap'n Proto message reader, refusing a
/// message of more than `max_words` words: packing collapses a run of zero
/// words into two bytes, so a small datagram could otherwise unpack into
/// megabytes (the traversal limit also bounds the segment sizes the header
/// may claim). Every decoder reads through here with its type's bound from
/// [`bounds`]; there is no unbounded reader (`cargo xtask check-capnp-bounds`).
pub(crate) fn unpack_bounded(
    data: &[u8],
    max_words: usize,
) -> Result<capnp::message::Reader<capnp::serialize::OwnedSegments>, CodecError> {
    let mut options = capnp::message::ReaderOptions::new();
    options.traversal_limit_in_words(Some(max_words));
    capnp::serialize_packed::read_message(data, options).map_err(|e| capnp_err(&e))
}

/// Read an optional text field, returning `None` when absent or empty.
fn text_or_none(
    has: bool,
    get: Result<capnp::text::Reader<'_>, capnp::Error>,
) -> Result<Option<String>, CodecError> {
    if !has {
        return Ok(None);
    }
    let s = text_to_string(get.map_err(|e| capnp_err(&e))?)?;
    Ok(if s.is_empty() { None } else { Some(s) })
}

/// Read an optional text field, returning an empty string when absent.
fn text_or_default(
    has: bool,
    get: Result<capnp::text::Reader<'_>, capnp::Error>,
) -> Result<String, CodecError> {
    if !has {
        return Ok(String::new());
    }
    text_to_string(get.map_err(|e| capnp_err(&e))?)
}

/// Read an optional bytes field, returning an empty vec when absent.
fn bytes_or_empty(has: bool, get: Result<&[u8], capnp::Error>) -> Result<Vec<u8>, CodecError> {
    if !has {
        return Ok(Vec::new());
    }
    Ok(get.map_err(|e| capnp_err(&e))?.to_vec())
}

/// Write a `GameInfo` into a capnp `GameStatus` builder.
fn write_game_status(
    mut gs: crate::presence_capnp::game_status::Builder<'_>,
    game: &presence::GameInfo,
) {
    gs.set_game_id(game.game_id);
    gs.set_game_name(&game.game_name);
    if let Some(ref si) = game.server_info {
        gs.set_server_info(si.as_str());
    }
    gs.set_elapsed_seconds(game.elapsed_seconds);
    if let Some(ref addr) = game.server_address {
        gs.set_server_address(addr.as_str());
    }
}

/// Read a capnp `GameStatus` reader into a `GameInfo`.
fn read_game_status(
    gs: crate::presence_capnp::game_status::Reader<'_>,
) -> Result<presence::GameInfo, CodecError> {
    Ok(presence::GameInfo {
        game_id: gs.get_game_id(),
        game_name: text_to_string(gs.get_game_name().map_err(|e| capnp_err(&e))?)?,
        server_info: text_or_none(gs.has_server_info(), gs.get_server_info())?,
        elapsed_seconds: gs.get_elapsed_seconds(),
        server_address: text_or_none(gs.has_server_address(), gs.get_server_address())?,
    })
}

pub mod account;
pub mod bounds;
pub mod chain;
pub mod direct;
pub mod friend;
pub mod identity;
pub mod presence;
pub mod transport_feedback;
pub mod voice;
pub mod voice_packet;

// community.capnp — V1 encode/decode removed (rekindle-server excluded from
// workspace). Community data now uses JSON via manifest.rs /
// member_registry.rs modules.

#[cfg(test)]
mod tests;
