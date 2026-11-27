//! A frontier request and its answer on the wire (plan E3.2 S7d-keys.8,
//! the unlock barrier): at unlock, before its first governance write in a
//! community, a device asks every known ring peer for that peer's own
//! `Frontier` (`governance_op.capnp`), its heads being the tips the
//! barrier walks for the highest seq of the device's key. The request is
//! signed by the asking member's pseudonym under
//! `domains::FRONTIER_REQUEST`, so the responder admits only a member; the
//! answer is the responder's own `Frontier` with at most
//! [`MAX_FRONTIER_REFS`] heads, its roster checksum and fork refs but no
//! fork proof ops (a reader fetches those by the refs), unsigned, since the
//! reader fetches and verifies every head it names from the head author's
//! own member record.

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::member_record::MAX_FRONTIER_REFS;
use rekindle_types::domains;
use rekindle_types::governance::Frontier;

use crate::capnp_codec::{bounds, capnp_err, pack, text_to_string, unpack_bounded};
use crate::checkpoint_state::bytes32;
use crate::community_governance_capnp::frontier_request;
use crate::governance_op::{decode_frontier, encode_frontier};
use crate::member_record::{check_len, check_size, put_lp, sign, verify};
use crate::CodecError;

/// Largest encoded request.
pub const FRONTIER_REQUEST_MAX_BYTES: usize = 384;

/// Largest encoded answer: [`MAX_FRONTIER_REFS`] longest heads, the
/// checksum and the fork refs, within the 32,768 B `app_call` cap with the
/// envelope around it.
pub const FRONTIER_RESPONSE_MAX_BYTES: usize = 31_000;

/// A community id (an anchor record key) at its longest text form, with room.
const MAX_COMMUNITY: usize = 160;

/// `requester` asks a ring peer of `community` for its frontier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontierRequest {
    pub community: String,
    pub requester: [u8; 32],
}

impl FrontierRequest {
    fn signing_bytes(&self) -> Result<Vec<u8>, CodecError> {
        let mut out = domains::FRONTIER_REQUEST.as_bytes().to_vec();
        put_lp(&mut out, self.community.as_bytes())?;
        out.extend_from_slice(&self.requester);
        Ok(out)
    }

    /// Sign by the requester's pseudonym `key` and encode.
    ///
    /// # Errors
    /// `key` is not the requester's, or the community id is over its cap.
    pub fn encode(&self, key: &SigningKey) -> Result<Vec<u8>, CodecError> {
        if key.verifying_key().to_bytes() != self.requester {
            return Err(CodecError::Serialization(
                "frontier request: the key is not the requester's".into(),
            ));
        }
        check_len("community id", self.community.len(), MAX_COMMUNITY)?;
        let sig = sign(key, &self.signing_bytes()?);
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut b = builder.init_root::<frontier_request::Builder<'_>>();
            b.set_community(&self.community);
            b.set_requester(&self.requester);
            b.set_sig(&sig);
        }
        let out = pack(&builder)?;
        check_size("frontier request", &out, FRONTIER_REQUEST_MAX_BYTES)?;
        Ok(out)
    }
}

/// Decode a request and verify its signature by the requester it names.
///
/// # Errors
/// Over the cap, malformed, or a bad signature.
pub fn decode_frontier_request(data: &[u8]) -> Result<FrontierRequest, CodecError> {
    check_size("frontier request", data, FRONTIER_REQUEST_MAX_BYTES)?;
    let reader = unpack_bounded(data, bounds::FRONTIER_REQUEST_BOUND)?;
    let b = reader
        .get_root::<frontier_request::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let community = text_to_string(b.get_community().map_err(|e| capnp_err(&e))?)?;
    check_len("community id", community.len(), MAX_COMMUNITY)?;
    let request = FrontierRequest {
        community,
        requester: bytes32(b.get_requester(), "frontier request: requester")?,
    };
    verify(
        &request.requester,
        &request.signing_bytes()?,
        b.get_sig().map_err(|e| capnp_err(&e))?,
        "frontier request",
    )?;
    Ok(request)
}

/// Encode the responder's own `frontier` as the answer.
///
/// # Errors
/// Over [`MAX_FRONTIER_REFS`] heads, fork proof ops present (the answer
/// names a fork by its refs only), or over [`FRONTIER_RESPONSE_MAX_BYTES`].
pub fn encode_frontier_response(frontier: &Frontier) -> Result<Vec<u8>, CodecError> {
    check_len("frontier heads", frontier.heads.len(), MAX_FRONTIER_REFS)?;
    if !frontier.fork_proof.is_empty() {
        return Err(CodecError::Serialization(
            "frontier response: fork proof ops travel by their refs".into(),
        ));
    }
    let out = encode_frontier(frontier)?;
    check_size("frontier response", &out, FRONTIER_RESPONSE_MAX_BYTES)?;
    Ok(out)
}

/// Decode an answer.
///
/// # Errors
/// Over the cap, malformed, over [`MAX_FRONTIER_REFS`] heads, or carrying
/// fork proof ops.
pub fn decode_frontier_response(data: &[u8]) -> Result<Frontier, CodecError> {
    check_size("frontier response", data, FRONTIER_RESPONSE_MAX_BYTES)?;
    let frontier = decode_frontier(data)?;
    if !frontier.fork_proof.is_empty() {
        return Err(CodecError::Deserialization(
            "frontier response: fork proof ops travel by their refs".into(),
        ));
    }
    Ok(frontier)
}

#[cfg(test)]
#[path = "frontier_request_tests.rs"]
mod tests;
