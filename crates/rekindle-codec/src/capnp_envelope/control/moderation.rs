//! `control` moderation variant encode/decode helpers.

use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community::envelope::ControlPayload;
use crate::community_envelope_capnp as cap;
use crate::error::CodecError;

pub(super) fn write_kick(mut p: cap::kick_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::Kick { target_pseudonym } = payload else {
        unreachable!("write_kick: variant mismatch")
    };
    p.set_target_pseudonym(target_pseudonym);
}

pub(super) fn read_kick(p: cap::kick_payload::Reader<'_>) -> Result<ControlPayload, CodecError> {
    Ok(ControlPayload::Kick {
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}
