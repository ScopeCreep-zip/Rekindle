//! `control` media variant encode/decode helpers.

use super::super::len_u32;
use super::{decode_control_payload, encode_control_payload};
use crate::capnp_codec::{capnp_err, not_in_schema, text_to_string};
use crate::community_envelope_capnp as cap;
use crate::dht::community::envelope::ControlPayload;
use crate::error::ProtocolError;
use rekindle_types::video::{Codec, ScalabilityMode};

/// Map the Rust `Codec` enum to its Cap'n Proto counterpart. Centralized
/// so adding a new variant is a single edit touching both sides.
fn codec_to_capnp(c: Codec) -> cap::Codec {
    match c {
        Codec::Vp9 => cap::Codec::Vp9,
        Codec::Vp8 => cap::Codec::Vp8,
        Codec::H264 => cap::Codec::H264,
    }
}

/// Inverse of [`codec_to_capnp`]. Returns an error on unknown wire
/// values (capnp's forward-compat `Unknown` discriminant); no fallback
/// (memory rule: `feedback_no_fallback`).
fn codec_from_capnp(c: cap::Codec) -> Codec {
    match c {
        cap::Codec::Vp9 => Codec::Vp9,
        cap::Codec::Vp8 => Codec::Vp8,
        cap::Codec::H264 => Codec::H264,
    }
}

fn scalability_mode_to_capnp(m: ScalabilityMode) -> cap::ScalabilityMode {
    match m {
        ScalabilityMode::Flat => cap::ScalabilityMode::Flat,
        ScalabilityMode::L1T2 => cap::ScalabilityMode::L1t2,
    }
}

fn scalability_mode_from_capnp(m: cap::ScalabilityMode) -> ScalabilityMode {
    match m {
        cap::ScalabilityMode::Flat => ScalabilityMode::Flat,
        cap::ScalabilityMode::L1t2 => ScalabilityMode::L1T2,
    }
}

pub(super) fn write_request_attachment(
    mut p: cap::request_attachment_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::RequestAttachment {
        channel_id,
        attachment_id,
        requested_chunks,
        requester_pseudonym,
    } = payload
    else {
        unreachable!("write_request_attachment: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_attachment_id(attachment_id);
    let mut list = p
        .reborrow()
        .init_requested_chunks(len_u32(requested_chunks.len()));
    for (i, c) in requested_chunks.iter().enumerate() {
        list.set(len_u32(i), *c);
    }
    p.set_requester_pseudonym(requester_pseudonym);
}

pub(super) fn write_attachment_chunk(
    mut p: cap::attachment_chunk_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::AttachmentChunk {
        attachment_id,
        chunk_index,
        data,
        plaintext_hash,
    } = payload
    else {
        unreachable!("write_attachment_chunk: variant mismatch")
    };
    p.set_attachment_id(attachment_id);
    p.set_chunk_index(*chunk_index);
    p.set_data(data);
    p.set_plaintext_hash(plaintext_hash);
}

pub(super) fn write_multi_attachment_chunk(
    mut p: cap::multi_attachment_chunk_payload::Builder<'_>,
    payload: &ControlPayload,
) -> Result<(), ProtocolError> {
    let ControlPayload::MultiAttachmentChunk { chunks } = payload else {
        unreachable!("write_multi_attachment_chunk: variant mismatch")
    };
    let mut list = p.reborrow().init_chunks(len_u32(chunks.len()));
    for (i, c) in chunks.iter().enumerate() {
        encode_control_payload(list.reborrow().get(len_u32(i)), c)?;
    }
    Ok(())
}

pub(super) fn write_video_fragment(
    mut p: cap::video_fragment_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::VideoFragment {
        channel_id,
        stream_id,
        frame_seq,
        frag_index,
        frag_total,
        keyframe,
        codec,
        timestamp,
        payload: data,
        signature,
    } = payload
    else {
        unreachable!("write_video_fragment: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_stream_id(stream_id);
    p.set_frame_seq(*frame_seq);
    p.set_frag_index(*frag_index);
    p.set_frag_total(*frag_total);
    p.set_keyframe(*keyframe);
    p.set_codec(codec_to_capnp(*codec));
    p.set_timestamp(*timestamp);
    p.set_payload(data);
    p.set_signature(signature);
}

pub(super) fn write_video_parity_fragment(
    mut p: cap::video_parity_fragment_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::VideoParityFragment {
        channel_id,
        stream_id,
        frame_seq,
        parity_index,
        parity_total,
        data_count,
        codec,
        frame_len,
        timestamp,
        payload: data,
        signature,
    } = payload
    else {
        unreachable!("write_video_parity_fragment: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_stream_id(stream_id);
    p.set_frame_seq(*frame_seq);
    p.set_parity_index(*parity_index);
    p.set_parity_total(*parity_total);
    p.set_data_count(*data_count);
    p.set_codec(codec_to_capnp(*codec));
    p.set_frame_len(*frame_len);
    p.set_timestamp(*timestamp);
    p.set_payload(data);
    p.set_signature(signature);
}

pub(super) fn write_frame_ack(
    mut p: cap::frame_ack_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::FrameAck {
        channel_id,
        stream_id,
        last_frame_seq,
        kbps,
        loss_q8,
    } = payload
    else {
        unreachable!("write_frame_ack: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_stream_id(stream_id);
    p.set_last_frame_seq(*last_frame_seq);
    p.set_kbps(*kbps);
    p.set_loss_q8(*loss_q8);
}

pub(super) fn write_keyframe_request(
    mut p: cap::keyframe_request_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::KeyframeRequest {
        channel_id,
        stream_id,
    } = payload
    else {
        unreachable!("write_keyframe_request: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_stream_id(stream_id);
}

pub(super) fn write_bandwidth_estimate(
    mut p: cap::bandwidth_estimate_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::BandwidthEstimate {
        channel_id,
        kbps,
        window_secs,
        loss_q8,
    } = payload
    else {
        unreachable!("write_bandwidth_estimate: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_kbps(*kbps);
    p.set_window_secs(*window_secs);
    p.set_loss_q8(*loss_q8);
}

pub(super) fn write_media_capabilities(
    mut p: cap::media_capabilities_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MediaCapabilities {
        channel_id,
        max_pixel_count,
        max_fps,
        encode_codecs,
        decode_codecs,
        supports_optimize_for_latency,
        supported_scalability_modes,
    } = payload
    else {
        unreachable!("write_media_capabilities: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_max_pixel_count(*max_pixel_count);
    p.set_max_fps(*max_fps);
    p.set_supports_optimize_for_latency(*supports_optimize_for_latency);
    {
        let mut list = p
            .reborrow()
            .init_encode_codecs(len_u32(encode_codecs.len()));
        for (i, c) in encode_codecs.iter().enumerate() {
            list.set(len_u32(i), codec_to_capnp(*c));
        }
    }
    {
        let mut list = p
            .reborrow()
            .init_decode_codecs(len_u32(decode_codecs.len()));
        for (i, c) in decode_codecs.iter().enumerate() {
            list.set(len_u32(i), codec_to_capnp(*c));
        }
    }
    let mut modes = p
        .reborrow()
        .init_supported_scalability_modes(len_u32(supported_scalability_modes.len()));
    for (i, m) in supported_scalability_modes.iter().enumerate() {
        modes.set(len_u32(i), scalability_mode_to_capnp(*m));
    }
}

pub(super) fn write_topology_change(
    mut p: cap::topology_change_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::TopologyChange {
        channel_id,
        stream_id,
        relay_host_pseudonym,
        reason,
        lamport,
    } = payload
    else {
        unreachable!("write_topology_change: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_stream_id(stream_id);
    p.set_has_relay_host(relay_host_pseudonym.is_some());
    if let Some(rh) = relay_host_pseudonym {
        p.set_relay_host_pseudonym(rh);
    }
    p.set_reason(reason);
    p.set_lamport(*lamport);
}

pub(super) fn read_request_attachment(
    p: cap::request_attachment_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let attachment_id_bytes = p.get_attachment_id().map_err(|e| capnp_err(&e))?;
    let attachment_id: [u8; 16] = attachment_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("attachment_id must be 16 bytes".into()))?;
    let chunks: Vec<u32> = p
        .get_requested_chunks()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .collect();
    Ok(ControlPayload::RequestAttachment {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        attachment_id,
        requested_chunks: chunks,
        requester_pseudonym: text_to_string(
            p.get_requester_pseudonym().map_err(|e| capnp_err(&e))?,
        )?,
    })
}

pub(super) fn read_attachment_chunk(
    p: cap::attachment_chunk_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let attachment_id_bytes = p.get_attachment_id().map_err(|e| capnp_err(&e))?;
    let attachment_id: [u8; 16] = attachment_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("attachment_id must be 16 bytes".into()))?;
    let plaintext_hash_bytes = p.get_plaintext_hash().map_err(|e| capnp_err(&e))?;
    let plaintext_hash: [u8; 32] = plaintext_hash_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("plaintext_hash must be 32 bytes".into()))?;
    Ok(ControlPayload::AttachmentChunk {
        attachment_id,
        chunk_index: p.get_chunk_index(),
        data: p.get_data().map_err(|e| capnp_err(&e))?.to_vec(),
        plaintext_hash,
    })
}

pub(super) fn read_multi_attachment_chunk(
    p: cap::multi_attachment_chunk_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let chunks: Result<Vec<ControlPayload>, ProtocolError> = p
        .get_chunks()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(decode_control_payload)
        .collect();
    Ok(ControlPayload::MultiAttachmentChunk { chunks: chunks? })
}

pub(super) fn read_video_fragment(
    p: cap::video_fragment_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let stream_id_bytes = p.get_stream_id().map_err(|e| capnp_err(&e))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("stream_id must be 16 bytes".into()))?;
    Ok(ControlPayload::VideoFragment {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        stream_id,
        frame_seq: p.get_frame_seq(),
        frag_index: p.get_frag_index(),
        frag_total: p.get_frag_total(),
        keyframe: p.get_keyframe(),
        codec: codec_from_capnp(p.get_codec().map_err(not_in_schema)?),
        timestamp: p.get_timestamp(),
        payload: p.get_payload().map_err(|e| capnp_err(&e))?.to_vec(),
        signature: p.get_signature().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

pub(super) fn read_video_parity_fragment(
    p: cap::video_parity_fragment_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let stream_id_bytes = p.get_stream_id().map_err(|e| capnp_err(&e))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("stream_id must be 16 bytes".into()))?;
    Ok(ControlPayload::VideoParityFragment {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        stream_id,
        frame_seq: p.get_frame_seq(),
        parity_index: p.get_parity_index(),
        parity_total: p.get_parity_total(),
        data_count: p.get_data_count(),
        codec: codec_from_capnp(p.get_codec().map_err(not_in_schema)?),
        frame_len: p.get_frame_len(),
        timestamp: p.get_timestamp(),
        payload: p.get_payload().map_err(|e| capnp_err(&e))?.to_vec(),
        signature: p.get_signature().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

pub(super) fn read_frame_ack(
    p: cap::frame_ack_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let stream_id_bytes = p.get_stream_id().map_err(|e| capnp_err(&e))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("stream_id must be 16 bytes".into()))?;
    Ok(ControlPayload::FrameAck {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        stream_id,
        last_frame_seq: p.get_last_frame_seq(),
        kbps: p.get_kbps(),
        loss_q8: p.get_loss_q8(),
    })
}

pub(super) fn read_keyframe_request(
    p: cap::keyframe_request_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let stream_id_bytes = p.get_stream_id().map_err(|e| capnp_err(&e))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("stream_id must be 16 bytes".into()))?;
    Ok(ControlPayload::KeyframeRequest {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        stream_id,
    })
}

pub(super) fn read_bandwidth_estimate(
    p: cap::bandwidth_estimate_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::BandwidthEstimate {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        kbps: p.get_kbps(),
        window_secs: p.get_window_secs(),
        loss_q8: p.get_loss_q8(),
    })
}

/// Decode one `List(Codec)` field into a typed `Vec<Codec>`.
fn read_codec_list(
    list: capnp::enum_list::Reader<'_, cap::Codec>,
) -> Result<Vec<Codec>, ProtocolError> {
    let mut codecs: Vec<Codec> = Vec::with_capacity(list.len() as usize);
    for c in list {
        codecs.push(codec_from_capnp(c.map_err(not_in_schema)?));
    }
    Ok(codecs)
}

pub(super) fn read_media_capabilities(
    p: cap::media_capabilities_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let encode_codecs = read_codec_list(p.get_encode_codecs().map_err(|e| capnp_err(&e))?)?;
    let decode_codecs = read_codec_list(p.get_decode_codecs().map_err(|e| capnp_err(&e))?)?;
    let mode_list = p
        .get_supported_scalability_modes()
        .map_err(|e| capnp_err(&e))?;
    let mut supported_scalability_modes: Vec<ScalabilityMode> =
        Vec::with_capacity(mode_list.len() as usize);
    for m in mode_list {
        supported_scalability_modes.push(scalability_mode_from_capnp(m.map_err(not_in_schema)?));
    }
    Ok(ControlPayload::MediaCapabilities {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        max_pixel_count: p.get_max_pixel_count(),
        max_fps: p.get_max_fps(),
        encode_codecs,
        decode_codecs,
        supports_optimize_for_latency: p.get_supports_optimize_for_latency(),
        supported_scalability_modes,
    })
}

pub(super) fn read_topology_change(
    p: cap::topology_change_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let stream_id_bytes = p.get_stream_id().map_err(|e| capnp_err(&e))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("stream_id must be 16 bytes".into()))?;
    Ok(ControlPayload::TopologyChange {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        stream_id,
        relay_host_pseudonym: if p.get_has_relay_host() {
            Some(text_to_string(
                p.get_relay_host_pseudonym().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        reason: text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}
