#[test]
fn a_voice_message_at_the_cap_passes_and_a_second_over_does_not() {
    let bytes = |samples: usize| vec![0u8; samples * 4];
    assert!(super::voice(&bytes(16_000 * 300)).is_ok(), "300.0 s");
    assert!(
        super::voice(&bytes(16_000 * 300 + 320)).is_ok(),
        "300 s + 1 frame"
    );
    assert!(super::voice(&bytes(16_000 * 301)).is_err(), "301 s");
}

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
    let voice = |pcm_bytes: usize| PinRequest::VoiceMessageSend {
        community: "VLD0:".to_owned() + &"A".repeat(43),
        channel_id: "c".into(),
        pcm: vec![0; pcm_bytes],
    };
    // 16 kHz f32: 64 bytes a millisecond.
    assert!(check(&voice(64 * 1_000)).is_ok());
    assert!(check(&voice(0)).is_err());
    assert!(check(&voice(64 * 199)).is_err());
    assert!(check(&voice(64 * 1_000 + 3)).is_err());
    // One 20 ms frame past five minutes is allowed, a millisecond more is not.
    assert!(check(&voice(64 * (5 * 60 * 1000 + 20))).is_ok());
    assert!(check(&voice(64 * (5 * 60 * 1000 + 21))).is_err());
}
