//! Pin and search requests (plan E3.1 E3b): the community key, the channel
//! and message ids, an attachment id of 16 bytes of hex, a bounded
//! search query, an absolute local path and a bounded voice message (E3d).

use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::protocol::PinRequest;
use rekindle_types::search::MAX_SEARCH_LIMIT;

use super::{community, Checked};

/// The longest channel or message id a pin names.
const MAX_PIN_ID_LEN: usize = 128;

/// The longest search query, in bytes.
pub(super) const MAX_SEARCH_QUERY_LEN: usize = 1024;

/// The longest local path an upload or download names, in bytes.
const MAX_PATH_LEN: usize = 4096;

fn id(value: &str, what: &str) -> Checked {
    if value.is_empty() || value.len() > MAX_PIN_ID_LEN {
        return Err(IpcResponse::error(
            400,
            format!("{what} must be 1–{MAX_PIN_ID_LEN} bytes"),
        ));
    }
    Ok(())
}

fn attachment_id(value: &str) -> Checked {
    match hex::decode(value) {
        Ok(bytes) if bytes.len() == 16 => Ok(()),
        _ => Err(IpcResponse::error(
            400,
            "invalid attachment id: must be 32 hex characters",
        )),
    }
}

/// A path the shell picked: absolute and of a bounded length. What is at it
/// is checked when the Host reads or writes it (`files::ops`).
fn local_path(value: &str) -> Checked {
    if value.len() > MAX_PATH_LEN || !std::path::Path::new(value).is_absolute() {
        return Err(IpcResponse::error(
            400,
            format!("a file path must be absolute and at most {MAX_PATH_LEN} bytes"),
        ));
    }
    Ok(())
}

/// A voice message's PCM: whole little-endian `f32` samples at the
/// Host's input rate, from the shortest to the longest voice message
/// (`rekindle_files::voice_note`, plan E5.1).
fn voice(pcm: &[u8]) -> Checked {
    use rekindle_files::voice_note::{INPUT_SAMPLE_RATE, MAX_ACCEPTED_MS, MIN_DURATION_MS};
    let bytes_for = |ms: u32| u64::from(ms) * u64::from(INPUT_SAMPLE_RATE) / 1000 * 4;
    let len = pcm.len() as u64;
    if !len.is_multiple_of(4)
        || len < bytes_for(MIN_DURATION_MS)
        || len > bytes_for(MAX_ACCEPTED_MS)
    {
        return Err(IpcResponse::error(
            400,
            format!(
                "a voice message is {MIN_DURATION_MS}–{MAX_ACCEPTED_MS} ms of f32 PCM at \
                 {INPUT_SAMPLE_RATE} Hz"
            ),
        ));
    }
    Ok(())
}

pub(super) fn check(request: &PinRequest) -> Checked {
    match request {
        PinRequest::ChannelPins {
            community: key,
            channel_id,
        } => {
            community(key)?;
            id(channel_id, "channel id")
        }
        PinRequest::MessagePin {
            community: key,
            channel_id,
            message_id,
        }
        | PinRequest::MessageUnpin {
            community: key,
            channel_id,
            message_id,
        } => {
            community(key)?;
            id(channel_id, "channel id")?;
            id(message_id, "message id")
        }
        PinRequest::AttachmentPin {
            community: key,
            attachment_id,
            ..
        } => {
            community(key)?;
            self::attachment_id(attachment_id)
        }
        PinRequest::AttachmentUpload {
            community: key,
            channel_id,
            path,
        } => {
            community(key)?;
            id(channel_id, "channel id")?;
            local_path(path)
        }
        PinRequest::AttachmentDownload {
            community: key,
            channel_id,
            attachment_id,
            path,
        } => {
            community(key)?;
            id(channel_id, "channel id")?;
            self::attachment_id(attachment_id)?;
            local_path(path)
        }
        PinRequest::AttachmentInfo {
            community: key,
            channel_id,
            attachment_id,
        }
        | PinRequest::VoiceMessageAudio {
            community: key,
            channel_id,
            attachment_id,
        } => {
            community(key)?;
            id(channel_id, "channel id")?;
            self::attachment_id(attachment_id)
        }
        PinRequest::VoiceMessageSend {
            community: key,
            channel_id,
            pcm,
        } => {
            community(key)?;
            id(channel_id, "channel id")?;
            voice(pcm)
        }
        PinRequest::MessageSearch { request } => {
            if request.query.len() > MAX_SEARCH_QUERY_LEN {
                return Err(IpcResponse::error(
                    400,
                    format!("a search query is at most {MAX_SEARCH_QUERY_LEN} bytes"),
                ));
            }
            if request.limit > MAX_SEARCH_LIMIT {
                return Err(IpcResponse::error(
                    400,
                    format!("a search returns at most {MAX_SEARCH_LIMIT} hits"),
                ));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "pins_tests.rs"]
mod tests;
