//! `control` bootstrap / sync variant encode/decode helpers.

use super::super::len_u32;
use super::super::sub_types::{
    read_bootstrap_channel_messages, read_channel_mek_delivery, read_member_info,
    read_synced_message, write_bootstrap_channel_messages, write_channel_mek_delivery,
    write_member_info, write_synced_message,
};
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_envelope_capnp as cap;
use crate::dht::community::envelope::ControlPayload;
use crate::error::ProtocolError;

pub(super) fn write_bootstrap_request(
    mut p: cap::bootstrap_request_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::BootstrapRequest {
        joiner_pseudonym,
        governance_key,
    } = payload
    else {
        unreachable!("write_bootstrap_request: variant mismatch")
    };
    p.set_joiner_pseudonym(joiner_pseudonym);
    p.set_governance_key(governance_key);
}

pub(super) fn write_bootstrap_response(
    mut p: cap::bootstrap_response_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::BootstrapResponse {
        governance_entries,
        member_list,
        channel_meks,
        recent_messages,
        wrapped_owner_keypair,
    } = payload
    else {
        unreachable!("write_bootstrap_response: variant mismatch")
    };
    let mut entries = p
        .reborrow()
        .init_governance_entries(len_u32(governance_entries.len()));
    for (i, e) in governance_entries.iter().enumerate() {
        super::super::governance::write_governance_entry(entries.reborrow().get(len_u32(i)), e);
    }
    let mut members = p.reborrow().init_member_list(len_u32(member_list.len()));
    for (i, m) in member_list.iter().enumerate() {
        write_member_info(members.reborrow().get(len_u32(i)), m);
    }
    let mut meks = p.reborrow().init_channel_meks(len_u32(channel_meks.len()));
    for (i, m) in channel_meks.iter().enumerate() {
        write_channel_mek_delivery(meks.reborrow().get(len_u32(i)), m);
    }
    let mut msgs = p
        .reborrow()
        .init_recent_messages(len_u32(recent_messages.len()));
    for (i, m) in recent_messages.iter().enumerate() {
        write_bootstrap_channel_messages(msgs.reborrow().get(len_u32(i)), m);
    }
    p.set_wrapped_owner_keypair(wrapped_owner_keypair);
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
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::BootstrapRequest {
        joiner_pseudonym: text_to_string(p.get_joiner_pseudonym().map_err(|e| capnp_err(&e))?)?,
        governance_key: text_to_string(p.get_governance_key().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_bootstrap_response(
    p: cap::bootstrap_response_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let governance_entries: Result<Vec<_>, ProtocolError> = p
        .get_governance_entries()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(super::super::governance::read_governance_entry)
        .collect();
    let member_list: Result<Vec<_>, ProtocolError> = p
        .get_member_list()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_member_info)
        .collect();
    let channel_meks: Result<Vec<_>, ProtocolError> = p
        .get_channel_meks()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_channel_mek_delivery)
        .collect();
    let recent_messages: Result<Vec<_>, ProtocolError> = p
        .get_recent_messages()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_bootstrap_channel_messages)
        .collect();
    Ok(ControlPayload::BootstrapResponse {
        governance_entries: governance_entries?,
        member_list: member_list?,
        channel_meks: channel_meks?,
        recent_messages: recent_messages?,
        wrapped_owner_keypair: p
            .get_wrapped_owner_keypair()
            .map_err(|e| capnp_err(&e))?
            .to_vec(),
    })
}

pub(super) fn read_sync_request(
    p: cap::sync_request_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::SyncRequest {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        since_timestamp: p.get_since_timestamp(),
    })
}

pub(super) fn read_sync_response(
    p: cap::sync_response_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let messages: Result<Vec<_>, ProtocolError> = p
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
