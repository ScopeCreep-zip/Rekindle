//! `control` bootstrap / sync variant encode/decode helpers.

use super::super::len_u32;
use super::super::sub_types::{read_synced_message, write_synced_message};
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community::envelope::ControlPayload;
use crate::community_envelope_capnp as cap;
use crate::error::CodecError;
use crate::governance_op::{read_op_ref, write_op_ref};
use rekindle_types::dht_layout::member_record::MAX_FRONTIER_REFS;

pub(super) fn write_bootstrap_request(
    mut p: cap::bootstrap_request_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::BootstrapRequest {
        joiner_pseudonym,
        anchor,
        member_joined,
        member_joined_op,
        joiner_route,
    } = payload
    else {
        unreachable!("write_bootstrap_request: variant mismatch")
    };
    p.set_joiner_pseudonym(joiner_pseudonym);
    p.set_anchor(anchor);
    write_op_ref(p.reborrow().init_member_joined(), member_joined);
    p.set_member_joined_op(member_joined_op);
    p.set_joiner_route(joiner_route);
}

/// Refuses a frontier over `MAX_FRONTIER_REFS` heads.
pub(super) fn write_bootstrap_response_v2(
    mut p: cap::bootstrap_response_v2_payload::Builder<'_>,
    payload: &ControlPayload,
) -> Result<(), CodecError> {
    let ControlPayload::BootstrapResponseV2 { anchor, frontier } = payload else {
        unreachable!("write_bootstrap_response_v2: variant mismatch")
    };
    check_bounds(frontier.len())?;
    p.set_anchor(anchor);
    let mut heads = p.init_frontier(len_u32(frontier.len()));
    for (i, head) in frontier.iter().enumerate() {
        write_op_ref(heads.reborrow().get(len_u32(i)), head);
    }
    Ok(())
}

fn check_bounds(frontier: usize) -> Result<(), CodecError> {
    if frontier > MAX_FRONTIER_REFS {
        return Err(CodecError::Deserialization(format!(
            "bootstrap response: {frontier} frontier heads (bound {MAX_FRONTIER_REFS})"
        )));
    }
    Ok(())
}

pub(super) fn write_sync_request(
    mut p: cap::sync_request_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::SyncRequest {
        channel_id,
        since_timestamp,
    } = payload
    else {
        unreachable!("write_sync_request: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_since_timestamp(*since_timestamp);
}

pub(super) fn write_sync_response(
    mut p: cap::sync_response_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::SyncResponse {
        channel_id,
        messages,
    } = payload
    else {
        unreachable!("write_sync_response: variant mismatch")
    };
    p.set_channel_id(channel_id);
    let mut list = p.reborrow().init_messages(len_u32(messages.len()));
    for (i, m) in messages.iter().enumerate() {
        write_synced_message(list.reborrow().get(len_u32(i)), m);
    }
}

pub(super) fn read_bootstrap_request(
    p: cap::bootstrap_request_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    Ok(ControlPayload::BootstrapRequest {
        joiner_pseudonym: text_to_string(p.get_joiner_pseudonym().map_err(|e| capnp_err(&e))?)?,
        anchor: text_to_string(p.get_anchor().map_err(|e| capnp_err(&e))?)?,
        member_joined: read_op_ref(p.get_member_joined().map_err(|e| capnp_err(&e))?)?,
        member_joined_op: p
            .get_member_joined_op()
            .map_err(|e| capnp_err(&e))?
            .to_vec(),
        joiner_route: p.get_joiner_route().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

pub(super) fn read_bootstrap_response_v2(
    p: cap::bootstrap_response_v2_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    let heads = p.get_frontier().map_err(|e| capnp_err(&e))?;
    check_bounds(usize::try_from(heads.len()).unwrap_or(usize::MAX))?;
    Ok(ControlPayload::BootstrapResponseV2 {
        anchor: text_to_string(p.get_anchor().map_err(|e| capnp_err(&e))?)?,
        frontier: heads.iter().map(read_op_ref).collect::<Result<_, _>>()?,
    })
}

pub(super) fn read_sync_request(
    p: cap::sync_request_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    Ok(ControlPayload::SyncRequest {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        since_timestamp: p.get_since_timestamp(),
    })
}

pub(super) fn read_sync_response(
    p: cap::sync_response_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    let messages: Result<Vec<_>, CodecError> = p
        .get_messages()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_synced_message)
        .collect();
    Ok(ControlPayload::SyncResponse {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        messages: messages?,
    })
}
