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
const MAX_SEARCH_QUERY_LEN: usize = 1024;

/// The longest local path an upload or download names, in bytes.
const MAX_PATH_LEN: usize = 4096;

/// The most waveform peaks a voice message carries.
const MAX_WAVEFORM_LEN: usize = 256;

/// The longest voice message, in milliseconds.
const MAX_VOICE_MS: u32 = 5 * 60 * 1000;

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

/// A voice message: Opus of at most the attachment cap, a waveform of at
/// most `MAX_WAVEFORM_LEN` peaks, and 1 ms to `MAX_VOICE_MS`.
fn voice(opus: &[u8], duration_ms: u32, waveform: &[u8]) -> Checked {
    if opus.is_empty() || opus.len() as u64 > rekindle_files::MAX_FILE_SIZE_BYTES {
        return Err(IpcResponse::error(
            400,
            "a voice message is 1 byte up to the attachment cap",
        ));
    }
    if duration_ms == 0 || duration_ms > MAX_VOICE_MS || waveform.len() > MAX_WAVEFORM_LEN {
        return Err(IpcResponse::error(
            400,
            format!(
                "a voice message lasts 1 ms to {MAX_VOICE_MS} ms with at most \
                 {MAX_WAVEFORM_LEN} waveform peaks"
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
            opus,
            duration_ms,
            waveform,
        } => {
            community(key)?;
            id(channel_id, "channel id")?;
            voice(opus, *duration_ms, waveform)
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
mod tests {
    use rekindle_types::search::{MessageSearch, SearchFilters, SearchSort};

    use super::*;

    fn search(query: String, limit: u32) -> PinRequest {
        PinRequest::MessageSearch {
            request: MessageSearch {
                query,
                filters: SearchFilters::default(),
                sort: SearchSort::Relevance,
                limit,
                offset: 0,
            },
        }
    }

    #[test]
    fn a_search_is_bounded() {
        assert!(check(&search("cargo".into(), 25)).is_ok());
        assert!(check(&search("x".repeat(MAX_SEARCH_QUERY_LEN + 1), 25)).is_err());
        assert!(check(&search("cargo".into(), MAX_SEARCH_LIMIT + 1)).is_err());
    }

    #[test]
    fn an_attachment_pin_names_sixteen_bytes() {
        let pin = |attachment_id: &str| PinRequest::AttachmentPin {
            community: "VLD0:".to_owned() + &"A".repeat(43),
            attachment_id: attachment_id.into(),
            pinned: true,
        };
        assert!(check(&pin("zz")).is_err());
        assert!(check(&pin(&"ab".repeat(15))).is_err());
    }

    #[test]
    fn an_upload_or_download_names_an_absolute_path() {
        let community = "VLD0:".to_owned() + &"A".repeat(43);
        let upload = |path: &str| PinRequest::AttachmentUpload {
            community: community.clone(),
            channel_id: "c".into(),
            path: path.into(),
        };
        assert!(check(&upload("/tmp/a.png")).is_ok());
        assert!(check(&upload("a.png")).is_err());
        assert!(check(&upload(&format!("/{}", "a".repeat(MAX_PATH_LEN)))).is_err());
        let download = PinRequest::AttachmentDownload {
            community: community.clone(),
            channel_id: "c".into(),
            attachment_id: "zz".into(),
            path: "/tmp/a.png".into(),
        };
        assert!(check(&download).is_err());
    }

    #[test]
    fn a_voice_message_is_bounded() {
        let voice = |opus: Vec<u8>, duration_ms: u32, peaks: usize| PinRequest::VoiceMessageSend {
            community: "VLD0:".to_owned() + &"A".repeat(43),
            channel_id: "c".into(),
            opus,
            duration_ms,
            waveform: vec![0; peaks],
        };
        assert!(check(&voice(vec![1; 16], 1_000, 64)).is_ok());
        assert!(check(&voice(Vec::new(), 1_000, 64)).is_err());
        assert!(check(&voice(vec![1; 16], 0, 64)).is_err());
        assert!(check(&voice(vec![1; 16], MAX_VOICE_MS + 1, 64)).is_err());
        assert!(check(&voice(vec![1; 16], 1_000, MAX_WAVEFORM_LEN + 1)).is_err());
    }
}
