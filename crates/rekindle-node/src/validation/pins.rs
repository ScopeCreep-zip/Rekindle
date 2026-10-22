//! Pin and search requests (plan E3.1 E3b): the community key, the channel
//! and message ids, an attachment id of 16 bytes of hex, and a bounded
//! search query.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::protocol::PinRequest;
use rekindle_types::search::MAX_SEARCH_LIMIT;

use super::{community, Checked};

/// The longest channel or message id a pin names.
const MAX_PIN_ID_LEN: usize = 128;

/// The longest search query, in bytes.
const MAX_SEARCH_QUERY_LEN: usize = 1024;

fn id(value: &str, what: &str) -> Checked {
    if value.is_empty() || value.len() > MAX_PIN_ID_LEN {
        return Err(IpcResponse::error(
            400,
            format!("{what} must be 1–{MAX_PIN_ID_LEN} bytes"),
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
            match hex::decode(attachment_id) {
                Ok(bytes) if bytes.len() == 16 => Ok(()),
                _ => Err(IpcResponse::error(
                    400,
                    "invalid attachment id: must be 32 hex characters",
                )),
            }
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
}
