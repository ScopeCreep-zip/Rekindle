//! A checkpoint's state snapshot on the wire (plan E3.2 S7c.3, "State
//! snapshot"): a member asks a ring peer for the bytes of
//! `canonical_state ‖ canonical_cut` at a checkpoint's cut, the preimage of
//! its `stateRoot`, in chunks. The request is signed by the asking member's
//! pseudonym under `domains::CHECKPOINT_STATE_REQUEST`, so the answering
//! member serves admitted members only; a joiner's request also carries a
//! tag keyed from the invite code ([`PreJoinInvite`]). The answer is
//! unsigned, since the reader checks the reassembled bytes against the
//! endorsed `stateRoot`.

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::invite::{hash_invite_code, pre_join_read_tag, pre_join_read_tag_verifies};
use rekindle_types::domains;

use crate::capnp_codec::{bounds, capnp_err, pack, text_to_string, unpack_bounded};
use crate::community_governance_capnp::{checkpoint_state_request, checkpoint_state_response};
use crate::member_record::{check_len, check_size, put_lp, sign, verify};
use crate::CodecError;

/// The most snapshot bytes one answer carries: under the 32,768 B
/// `app_call` cap with the envelope around it (plan E3.2 S7c.3).
pub const SNAPSHOT_CHUNK_MAX: usize = 30_000;

/// Largest encoded request.
pub const CHECKPOINT_STATE_REQUEST_MAX_BYTES: usize = 512;

/// Largest encoded answer: a full chunk and its framing.
pub const CHECKPOINT_STATE_RESPONSE_MAX_BYTES: usize = SNAPSHOT_CHUNK_MAX + 512;

/// A community id (an anchor record key) at its longest text form, with room.
const MAX_COMMUNITY: usize = 160;

/// An invite code hash (64 hex characters), with room.
const MAX_CODE_HASH: usize = 128;

/// A joiner's authority before it is a member (plan E3.2 S7c.5, S7c.8
/// (2)): the lookup id of the invite it holds, signed with the request, and
/// [`pre_join_read_tag`] under the invite code over the request's signed
/// bytes. A Veilid `app_call` names no sender and the pseudonym is the
/// asker's own choice, so the tag, which only a holder of the code (never
/// in the op log) can make, is what admits a joiner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreJoinInvite {
    pub code_hash: String,
    pub tag: [u8; 32],
}

impl PreJoinInvite {
    /// Append `0x00`, or `0x01 ‖ len ‖ code hash`, to a request's signed
    /// bytes.
    pub(crate) fn put(invite: Option<&Self>, out: &mut Vec<u8>) -> Result<(), CodecError> {
        match invite {
            None => out.push(0),
            Some(invite) => {
                out.push(1);
                put_lp(out, invite.code_hash.as_bytes())?;
            }
        }
        Ok(())
    }

    /// The invite of `code` for a request whose signed bytes, once it names
    /// the code's lookup id, `signed` returns.
    pub(crate) fn of_code(
        code: &str,
        signed: impl FnOnce(&Self) -> Result<Vec<u8>, CodecError>,
    ) -> Result<Self, CodecError> {
        let mut invite = Self {
            code_hash: hash_invite_code(code),
            tag: [0; 32],
        };
        invite.tag = pre_join_read_tag(code, &signed(&invite)?);
        Ok(invite)
    }

    /// Whether `code` is this invite's code and tagged `signed`.
    #[must_use]
    pub fn proven_by(&self, code: &str, signed: &[u8]) -> bool {
        self.code_hash == hash_invite_code(code)
            && pre_join_read_tag_verifies(code, signed, &self.tag)
    }

    /// Check an encoded invite's fields before writing them: the code hash
    /// and tag text, empty for a member.
    pub(crate) fn wire(invite: Option<&Self>) -> Result<(&str, &[u8]), CodecError> {
        match invite {
            None => Ok(("", &[])),
            Some(invite) if invite.code_hash.is_empty() => Err(CodecError::Serialization(
                "pre-join request: an empty invite code hash".into(),
            )),
            Some(invite) => {
                check_len("invite code hash", invite.code_hash.len(), MAX_CODE_HASH)?;
                Ok((&invite.code_hash, &invite.tag))
            }
        }
    }

    /// Read a decoded request's invite: `None` when both fields are empty,
    /// else a code hash within its cap and a 32-byte tag.
    pub(crate) fn read(
        code_hash: Result<capnp::text::Reader<'_>, capnp::Error>,
        tag: Result<&[u8], capnp::Error>,
    ) -> Result<Option<Self>, CodecError> {
        let code_hash = text_to_string(code_hash.map_err(|e| capnp_err(&e))?)?;
        let tag = tag.map_err(|e| capnp_err(&e))?;
        if code_hash.is_empty() && tag.is_empty() {
            return Ok(None);
        }
        check_len("invite code hash", code_hash.len(), MAX_CODE_HASH)?;
        if code_hash.is_empty() {
            return Err(CodecError::Deserialization(
                "pre-join request: a tag without an invite code hash".into(),
            ));
        }
        Ok(Some(Self {
            code_hash,
            tag: bytes32(Ok(tag), "pre-join request: invite tag")?,
        }))
    }
}

/// A request a joiner may ask before it is a member (the checkpoint hint and
/// snapshot requests): its signed bytes and its invite, and through them
/// the tag a joiner puts on it and the inviter checks.
pub trait PreJoinRequest: Clone {
    /// The bytes the requester signs, which name the invite's code hash.
    fn signed_bytes(&self) -> Result<Vec<u8>, CodecError>;
    /// The invite a joiner's request carries.
    fn invite_slot(&mut self) -> &mut Option<PreJoinInvite>;
    /// The invite, read-only.
    fn invite_ref(&self) -> Option<&PreJoinInvite>;

    /// This request as a joiner holding the invite `code` asks it.
    fn with_invite_code(mut self, code: &str) -> Result<Self, CodecError> {
        let invite = PreJoinInvite::of_code(code, |invite| {
            let mut named = self.clone();
            *named.invite_slot() = Some(invite.clone());
            named.signed_bytes()
        })?;
        *self.invite_slot() = Some(invite);
        Ok(self)
    }

    /// Whether this request carries the invite `code`'s tag.
    fn invite_proven_by(&self, code: &str) -> Result<bool, CodecError> {
        let Some(invite) = self.invite_ref() else {
            return Ok(false);
        };
        Ok(invite.proven_by(code, &self.signed_bytes()?))
    }
}

impl PreJoinRequest for CheckpointStateRequest {
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

/// `requester` asks for the snapshot bytes at `offset` of the checkpoint
/// with this `height` and `digest` in `community`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointStateRequest {
    pub community: String,
    pub height: u64,
    pub digest: [u8; 32],
    pub offset: u32,
    pub requester: [u8; 32],
    /// `None`: a member asks. `Some`: the live invite a joiner holds, its
    /// authority before it is a member (plan E3.2 S7c.5, S7c.8 (2)); set by
    /// [`Self::with_invite_code`].
    pub invite: Option<PreJoinInvite>,
}

impl CheckpointStateRequest {
    fn signing_bytes(&self) -> Result<Vec<u8>, CodecError> {
        let mut out = domains::CHECKPOINT_STATE_REQUEST.as_bytes().to_vec();
        put_lp(&mut out, self.community.as_bytes())?;
        out.extend_from_slice(&self.height.to_be_bytes());
        out.extend_from_slice(&self.digest);
        out.extend_from_slice(&self.offset.to_be_bytes());
        out.extend_from_slice(&self.requester);
        PreJoinInvite::put(self.invite.as_ref(), &mut out)?;
        Ok(out)
    }

    /// Sign by the requester's pseudonym `key` and encode.
    ///
    /// # Errors
    /// `key` is not the requester's, or the community id or the request is
    /// over its cap.
    pub fn encode(&self, key: &SigningKey) -> Result<Vec<u8>, CodecError> {
        if key.verifying_key().to_bytes() != self.requester {
            return Err(CodecError::Serialization(
                "checkpoint state request: the key is not the requester's".into(),
            ));
        }
        check_len("community id", self.community.len(), MAX_COMMUNITY)?;
        let (code_hash, tag) = PreJoinInvite::wire(self.invite.as_ref())?;
        let sig = sign(key, &self.signing_bytes()?);
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut b = builder.init_root::<checkpoint_state_request::Builder<'_>>();
            b.set_community(&self.community);
            b.set_height(self.height);
            b.set_digest(&self.digest);
            b.set_offset(self.offset);
            b.set_requester(&self.requester);
            b.set_sig(&sig);
            b.set_invite_code_hash(code_hash);
            b.set_invite_tag(tag);
        }
        let out = pack(&builder)?;
        check_size(
            "checkpoint state request",
            &out,
            CHECKPOINT_STATE_REQUEST_MAX_BYTES,
        )?;
        Ok(out)
    }
}

pub(crate) fn bytes32(
    data: Result<&[u8], capnp::Error>,
    what: &str,
) -> Result<[u8; 32], CodecError> {
    data.map_err(|e| capnp_err(&e))?
        .try_into()
        .map_err(|_| CodecError::Deserialization(format!("{what} is not 32 bytes")))
}

/// Decode a request and verify its signature by the requester it names.
///
/// # Errors
/// Over the cap, malformed, or a bad signature.
pub fn decode_checkpoint_state_request(data: &[u8]) -> Result<CheckpointStateRequest, CodecError> {
    check_size(
        "checkpoint state request",
        data,
        CHECKPOINT_STATE_REQUEST_MAX_BYTES,
    )?;
    let reader = unpack_bounded(data, bounds::CHECKPOINT_STATE_REQUEST_BOUND)?;
    let b = reader
        .get_root::<checkpoint_state_request::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let request = CheckpointStateRequest {
        community: text_to_string(b.get_community().map_err(|e| capnp_err(&e))?)?,
        height: b.get_height(),
        digest: bytes32(b.get_digest(), "checkpoint state request: digest")?,
        offset: b.get_offset(),
        requester: bytes32(b.get_requester(), "checkpoint state request: requester")?,
        invite: PreJoinInvite::read(b.get_invite_code_hash(), b.get_invite_tag())?,
    };
    verify(
        &request.requester,
        &request.signing_bytes()?,
        b.get_sig().map_err(|e| capnp_err(&e))?,
        "checkpoint state request",
    )?;
    Ok(request)
}

/// At most [`SNAPSHOT_CHUNK_MAX`] of the `total` snapshot bytes, from the
/// request's offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointStateResponse {
    pub total: u32,
    pub bytes: Vec<u8>,
}

impl CheckpointStateResponse {
    /// Encode.
    ///
    /// # Errors
    /// The chunk is over [`SNAPSHOT_CHUNK_MAX`].
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        check_len("snapshot chunk", self.bytes.len(), SNAPSHOT_CHUNK_MAX)?;
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut b = builder.init_root::<checkpoint_state_response::Builder<'_>>();
            b.set_total(self.total);
            b.set_bytes(&self.bytes);
        }
        let out = pack(&builder)?;
        check_size(
            "checkpoint state response",
            &out,
            CHECKPOINT_STATE_RESPONSE_MAX_BYTES,
        )?;
        Ok(out)
    }
}

/// Decode an answer.
///
/// # Errors
/// Over the cap, malformed, or a chunk over [`SNAPSHOT_CHUNK_MAX`].
pub fn decode_checkpoint_state_response(
    data: &[u8],
) -> Result<CheckpointStateResponse, CodecError> {
    check_size(
        "checkpoint state response",
        data,
        CHECKPOINT_STATE_RESPONSE_MAX_BYTES,
    )?;
    let reader = unpack_bounded(data, bounds::CHECKPOINT_STATE_RESPONSE_BOUND)?;
    let b = reader
        .get_root::<checkpoint_state_response::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let bytes = b.get_bytes().map_err(|e| capnp_err(&e))?.to_vec();
    if bytes.len() > SNAPSHOT_CHUNK_MAX {
        return Err(CodecError::Deserialization(format!(
            "snapshot chunk is {} bytes, the bound is {SNAPSHOT_CHUNK_MAX}",
            bytes.len()
        )));
    }
    Ok(CheckpointStateResponse {
        total: b.get_total(),
        bytes,
    })
}

#[cfg(test)]
#[path = "checkpoint_state_tests.rs"]
mod tests;
