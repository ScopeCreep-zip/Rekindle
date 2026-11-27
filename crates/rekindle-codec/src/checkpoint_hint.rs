//! A checkpoint hint on the wire (plan E3.2 S7c.5, "Using a checkpoint"):
//! before its walk a joiner asks its inviter for the usable hint, the
//! hint's `Checkpoint` op and the endorsements a strict majority needs,
//! then, height by height, the hint each `prevCheckpoint` names, so it
//! checks the endorsement chain down to the `GenesisCert`. The request is
//! signed by the asker's pseudonym under `domains::CHECKPOINT_HINT_REQUEST`
//! and carries the invite a joiner holds with its tag
//! ([`crate::checkpoint_state::PreJoinInvite`]), as the snapshot request
//! does; the answer is unsigned, since the reader verifies every op
//! it fetches by the refs.

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::domains;
use rekindle_types::governance::OpRef;

use crate::capnp_codec::{bounds, capnp_err, pack, text_to_string, unpack_bounded};
use crate::checkpoint_state::{bytes32, PreJoinInvite, PreJoinRequest};
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

/// Largest encoded answer: [`MAX_HINT_ENDORSEMENTS`] + 1 longest locators,
/// each naming its op's 32-byte device key (ADR 0022 rule 1), within the
/// `app_call` cap with the envelope around it.
pub const CHECKPOINT_HINT_RESPONSE_MAX_BYTES: usize = 31_000;

/// A community id (an anchor record key) at its longest text form, with room.
const MAX_COMMUNITY: usize = 160;

/// `requester` asks for the hint selected at the greatest height
/// (`height` 0), or the usable hint at `height` carrying `digest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointHintRequest {
    pub community: String,
    pub height: u64,
    pub digest: [u8; 32],
    pub requester: [u8; 32],
    /// `None`: a member asks. `Some`: the live invite a joiner holds, set
    /// by [`Self::with_invite_code`].
    pub invite: Option<PreJoinInvite>,
}

impl CheckpointHintRequest {
    fn signing_bytes(&self) -> Result<Vec<u8>, CodecError> {
        let mut out = domains::CHECKPOINT_HINT_REQUEST.as_bytes().to_vec();
        put_lp(&mut out, self.community.as_bytes())?;
        out.extend_from_slice(&self.height.to_be_bytes());
        out.extend_from_slice(&self.digest);
        out.extend_from_slice(&self.requester);
        PreJoinInvite::put(self.invite.as_ref(), &mut out)?;
        Ok(out)
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
        let (code_hash, tag) = PreJoinInvite::wire(self.invite.as_ref())?;
        let sig = sign(key, &self.signing_bytes()?);
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut b = builder.init_root::<checkpoint_hint_request::Builder<'_>>();
            b.set_community(&self.community);
            b.set_height(self.height);
            b.set_digest(&self.digest);
            b.set_requester(&self.requester);
            b.set_invite_code_hash(code_hash);
            b.set_invite_tag(tag);
            b.set_sig(&sig);
        }
        let out = pack(&builder)?;
        check_size(
            "checkpoint hint request",
            &out,
            CHECKPOINT_HINT_REQUEST_MAX_BYTES,
        )?;
        Ok(out)
    }
}

impl PreJoinRequest for CheckpointHintRequest {
    fn signed_bytes(&self) -> Result<Vec<u8>, CodecError> {
        self.signing_bytes()
    }
    fn invite_slot(&mut self) -> &mut Option<PreJoinInvite> {
        &mut self.invite
    }
    fn invite_ref(&self) -> Option<&PreJoinInvite> {
        self.invite.as_ref()
    }
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
        invite: PreJoinInvite::read(b.get_invite_code_hash(), b.get_invite_tag())?,
    };
    verify(
        &request.requester,
        &request.signing_bytes()?,
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
        let out = pack(&builder)?;
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
