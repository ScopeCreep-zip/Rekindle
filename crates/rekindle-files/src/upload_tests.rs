#![allow(
    clippy::cast_possible_truncation,
    reason = "test fixture: the constants are compile-time known and well inside range; a fallible conversion here would add noise without adding safety"
)]

use super::*;
use crate::test_mock::MockDeps;

/// A minimal valid upload request for the rejection-path tests, where
/// the gate fires before the payload matters.
fn sample_request() -> UploadRequest {
    UploadRequest {
        community_id: "c1".into(),
        channel_id: "11111111111111111111111111111111".into(),
        bytes: b"x".to_vec(),
        filename: "f.txt".into(),
        mime_type: "text/plain".into(),
        body_plaintext: Vec::new(),
        flags: 0,
        local_path: None,
    }
}

#[tokio::test]
async fn happy_path_queues_message_and_persists_row() {
    let deps = MockDeps::new("c1", "11111111111111111111111111111111").with_mek(7, [42u8; 32]);
    let bytes = b"hello world payload".to_vec();
    let result = upload_bytes_as_attachment(
        &deps,
        UploadRequest {
            community_id: "c1".into(),
            channel_id: "11111111111111111111111111111111".into(),
            bytes,
            filename: "hello.txt".into(),
            mime_type: "text/plain".into(),
            body_plaintext: Vec::new(),
            flags: 0,
            local_path: Some("/tmp/hello.txt".into()),
        },
    )
    .await;
    assert!(result.is_ok(), "happy path should succeed");
    let attachment_id_hex = result.unwrap();
    assert_eq!(attachment_id_hex.len(), 32, "16-byte UUID hex");

    let calls = deps.calls.lock();
    assert_eq!(
        calls.posts_queued.len(),
        1,
        "one channel_post row, no possession entry of the uploader's"
    );
    assert_eq!(calls.channel_messages_persisted.len(), 1, "one SQLite row");
    assert_eq!(
        calls.slowmode_persists.len(),
        1,
        "slowmode timestamp persisted"
    );
}

#[tokio::test]
async fn oversize_file_rejected() {
    let deps = MockDeps::new("c1", "11111111111111111111111111111111").with_mek(1, [0u8; 32]);
    // MAX_FILE_SIZE_BYTES = 28*1024*1000 ≈ 28 MB. Build a 1-byte-larger one.
    let bytes = vec![0u8; (MAX_FILE_SIZE_BYTES + 1) as usize];
    let err = upload_bytes_as_attachment(
        &deps,
        UploadRequest {
            community_id: "c1".into(),
            channel_id: "11111111111111111111111111111111".into(),
            bytes,
            filename: "big.bin".into(),
            mime_type: "application/octet-stream".into(),
            body_plaintext: Vec::new(),
            flags: 0,
            local_path: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, FilesError::FileTooLarge { .. }));
}

#[tokio::test]
async fn forum_channel_rejected() {
    let mut deps = MockDeps::new("c1", "11111111111111111111111111111111").with_mek(1, [0u8; 32]);
    deps.forum_channel = true;
    let err = upload_bytes_as_attachment(&deps, sample_request())
        .await
        .unwrap_err();
    assert!(matches!(err, FilesError::InvalidInput(msg) if msg.contains("forum")));
}

#[tokio::test]
async fn permission_denied_returns_error() {
    let mut deps = MockDeps::new("c1", "11111111111111111111111111111111").with_mek(1, [0u8; 32]);
    deps.permission_pass = false;
    let err = upload_bytes_as_attachment(&deps, sample_request())
        .await
        .unwrap_err();
    assert!(matches!(err, FilesError::PermissionDenied(_)));
}

#[tokio::test]
async fn slowmode_blocks_upload() {
    let mut deps = MockDeps::new("c1", "11111111111111111111111111111111").with_mek(1, [0u8; 32]);
    deps.slowmode_pass = false;
    let err = upload_bytes_as_attachment(&deps, sample_request())
        .await
        .unwrap_err();
    assert!(matches!(err, FilesError::Slowmode(_)));
}

#[test]
fn guess_mime_type_matches_extensions() {
    use std::path::PathBuf;
    assert_eq!(guess_mime_type(&PathBuf::from("a.png")), "image/png");
    assert_eq!(guess_mime_type(&PathBuf::from("a.OGG")), "audio/ogg");
    assert_eq!(
        guess_mime_type(&PathBuf::from("a.unknown")),
        "application/octet-stream"
    );
    assert_eq!(
        guess_mime_type(&PathBuf::from("noext")),
        "application/octet-stream"
    );
}
