//! A checkpoint hint on the wire (plan E3.2 S7c.5, "Using a checkpoint"):
//! before its walk a joiner asks its inviter for the usable hint, the
//! hint's `Checkpoint` op and the endorsements a strict majority needs,
//! then, height by height, the hint each `prevCheckpoint` names, so it
//! checks the endorsement chain down to the `GenesisCert`. The request is
//! signed by the asker's pseudonym under `domains::CHECKPOINT_HINT_REQUEST`
//! and carries the code hash of the invite a joiner holds, as the snapshot
//! request does; the answer is unsigned, since the reader verifies every op
//! it fetches by the refs.

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::domains;
use rekindle_types::governance::OpRef;

use crate::capnp_codec::{bounds, capnp_err, pack, text_to_string, unpack_bounded};
use crate::community_governance_capnp::{checkpoint_hint_request, checkpoint_hint_response};
use crate::governance_op::{read_op_ref, write_op_ref};
use crate::member_record::{check_len, check_size, put_lp, sign, verify};
use crate::CodecError;

/// Largest encoded request.
pub const CHECKPOINT_HINT_REQUEST_MAX_BYTES: usize = 512;

/// The most endorsements one answer lists: a strict majority of an
/// electorate of up to 255 members, under the 32,768 B `app_call` cap with
/// the envelope around it. A hint whose majority needs more is not
/// answered, and the joiner replays in full.
pub const MAX_HINT_ENDORSEMENTS: usize = 128;

/// Largest encoded answer.
pub const CHECKPOINT_HINT_RESPONSE_MAX_BYTES: usize = 30_000;

/// A community id (an anchor record key) at its longest text form, with room.
const MAX_COMMUNITY: usize = 160;

/// An invite code hash (64 hex characters), with room.
const MAX_CODE_HASH: usize = 128;

/// `requester` asks for the hint selected at the greatest height
/// (`height` 0), or the usable hint at `height` carrying `digest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointHintRequest {
    pub community: String,
    pub height: u64,
    pub digest: [u8; 32],
    pub requester: [u8; 32],
    /// `None`: a member asks. `Some`: the code hash of the live invite a
    /// joiner holds; signed with the rest.
    pub invite_code_hash: Option<String>,
}

impl CheckpointHintRequest {
    fn signing_bytes(&self) -> Vec<u8> {
        let mut out = domains::CHECKPOINT_HINT_REQUEST.as_bytes().to_vec();
        put_lp(&mut out, self.community.as_bytes());
        out.extend_from_slice(&self.height.to_be_bytes());
        out.extend_from_slice(&self.digest);
        out.extend_from_slice(&self.requester);
        match &self.invite_code_hash {
            None => out.push(0),
            Some(hash) => {
                out.push(1);
                put_lp(&mut out, hash.as_bytes());
            }
        }
        out
    }

    /// Sign by the requester's pseudonym `key` and encode.
    ///
    /// # Errors
    /// `key` is not the requester's, a field is over its cap, or the
    /// invite code hash is empty.
    pub fn encode(&self, key: &SigningKey) -> Result<Vec<u8>, CodecError> {
        if key.verifying_key().to_bytes() != self.requester {
            return Err(CodecError::Serialization(
                "checkpoint hint request: the key is not the requester's".into(),
            ));
        }
        check_len("community id", self.community.len(), MAX_COMMUNITY)?;
        let code_hash = self.invite_code_hash.as_deref().unwrap_or_default();
        check_len("invite code hash", code_hash.len(), MAX_CODE_HASH)?;
        if self.invite_code_hash.as_deref() == Some("") {
            return Err(CodecError::Serialization(
                "checkpoint hint request: an empty invite code hash".into(),
            ));
        }
        let sig = sign(key, &self.signing_bytes());
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut b = builder.init_root::<checkpoint_hint_request::Builder<'_>>();
            b.set_community(&self.community);
            b.set_height(self.height);
            b.set_digest(&self.digest);
            b.set_requester(&self.requester);
            b.set_invite_code_hash(code_hash);
            b.set_sig(&sig);
        }
        let out = pack(&builder);
        check_size(
            "checkpoint hint request",
            &out,
            CHECKPOINT_HINT_REQUEST_MAX_BYTES,
        )?;
        Ok(out)
    }
}

fn bytes32(data: Result<&[u8], capnp::Error>, what: &str) -> Result<[u8; 32], CodecError> {
    data.map_err(|e| capnp_err(&e))?
        .try_into()
        .map_err(|_| CodecError::Deserialization(format!("{what} is not 32 bytes")))
}

/// Decode a request and verify its signature by the requester it names.
///
/// # Errors
/// Over the cap, malformed, or a bad signature.
pub fn decode_checkpoint_hint_request(data: &[u8]) -> Result<CheckpointHintRequest, CodecError> {
    check_size(
        "checkpoint hint request",
        data,
        CHECKPOINT_HINT_REQUEST_MAX_BYTES,
    )?;
    let reader = unpack_bounded(data, bounds::CHECKPOINT_HINT_REQUEST_BOUND)?;
    let b = reader
        .get_root::<checkpoint_hint_request::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let request = CheckpointHintRequest {
        community: text_to_string(b.get_community().map_err(|e| capnp_err(&e))?)?,
        height: b.get_height(),
        digest: bytes32(b.get_digest(), "checkpoint hint request: digest")?,
        requester: bytes32(b.get_requester(), "checkpoint hint request: requester")?,
        invite_code_hash: Some(text_to_string(
            b.get_invite_code_hash().map_err(|e| capnp_err(&e))?,
        )?)
        .filter(|hash| !hash.is_empty()),
    };
    verify(
        &request.requester,
        &request.signing_bytes(),
        b.get_sig().map_err(|e| capnp_err(&e))?,
        "checkpoint hint request",
    )?;
    Ok(request)
}

/// The hint's lowest `Checkpoint` op and the endorsements a strict
/// majority of its electorate needs (`None` and empty: no usable hint).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckpointHintResponse {
    pub checkpoint: Option<OpRef>,
    pub endorsements: Vec<OpRef>,
}

impl CheckpointHintResponse {
    /// Encode.
    ///
    /// # Errors
    /// Over [`MAX_HINT_ENDORSEMENTS`] endorsements, endorsements with no
    /// checkpoint, or over the cap.
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        check_len(
            "hint endorsements",
            self.endorsements.len(),
            MAX_HINT_ENDORSEMENTS,
        )?;
        if self.checkpoint.is_none() && !self.endorsements.is_empty() {
            return Err(CodecError::Serialization(
                "checkpoint hint response: endorsements with no checkpoint".into(),
            ));
        }
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut b = builder.init_root::<checkpoint_hint_response::Builder<'_>>();
            b.set_has_checkpoint(self.checkpoint.is_some());
            if let Some(r) = &self.checkpoint {
                write_op_ref(b.reborrow().init_checkpoint_ref(), r);
            }
            let len = u32::try_from(self.endorsements.len())
                .map_err(|_| CodecError::Serialization("too many endorsements".into()))?;
            let mut list = b.init_endorsements(len);
            for (i, r) in (0..len).zip(&self.endorsements) {
                write_op_ref(list.reborrow().get(i), r);
            }
        }
        let out = pack(&builder);
        check_size(
            "checkpoint hint response",
            &out,
            CHECKPOINT_HINT_RESPONSE_MAX_BYTES,
        )?;
        Ok(out)
    }
}

/// Decode an answer.
///
/// # Errors
/// Over the cap, malformed, or over [`MAX_HINT_ENDORSEMENTS`].
pub fn decode_checkpoint_hint_response(data: &[u8]) -> Result<CheckpointHintResponse, CodecError> {
    check_size(
        "checkpoint hint response",
        data,
        CHECKPOINT_HINT_RESPONSE_MAX_BYTES,
    )?;
    let reader = unpack_bounded(data, bounds::CHECKPOINT_HINT_RESPONSE_BOUND)?;
    let b = reader
        .get_root::<checkpoint_hint_response::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let list = b.get_endorsements().map_err(|e| capnp_err(&e))?;
    check_len(
        "hint endorsements",
        list.len() as usize,
        MAX_HINT_ENDORSEMENTS,
    )?;
    let endorsements = list
        .iter()
        .map(read_op_ref)
        .collect::<Result<Vec<_>, _>>()?;
    let checkpoint = if b.get_has_checkpoint() {
        Some(read_op_ref(
            b.get_checkpoint_ref().map_err(|e| capnp_err(&e))?,
        )?)
    } else if endorsements.is_empty() {
        None
    } else {
        return Err(CodecError::Deserialization(
            "checkpoint hint response: endorsements with no checkpoint".into(),
        ));
    };
    Ok(CheckpointHintResponse {
        checkpoint,
        endorsements,
    })
}

#[cfg(test)]
#[path = "checkpoint_hint_tests.rs"]
mod tests;
