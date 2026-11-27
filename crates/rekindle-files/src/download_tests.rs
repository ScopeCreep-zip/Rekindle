use super::*;
use crate::test_mock::MockDeps;
use rekindle_codec::community::channel_record::{ChannelMessage, ChannelRecordEntry};

fn build_offer_entry(attachment_id: [u8; 16], chunk_count: u32) -> ChannelRecordEntry {
    ChannelRecordEntry::Message(ChannelMessage {
        sequence: 1,
        sender_pseudonym: "owner".into(),
        ciphertext: Vec::new(),
        mek_generation: 0,
        timestamp: 0,
        reply_to: None,
        revives: None,
        lamport_ts: 1,
        message_id: Some("m1".into()),
        attachment: Some(AttachmentOffer {
            attachment_id,
            filename: "f.bin".into(),
            mime_type: "application/octet-stream".into(),
            total_size: 0,
            chunk_count,
            chunk_size: 0,
            merkle_root: [0u8; 32],
            chunk_hashes: vec![[0u8; 32]; chunk_count as usize],
            wrapped_fek: Vec::new(),
            fek_mek_generation: 0,
        }),
        flags: 0,
        mentioned_pseudonyms: Vec::new(),
        mentioned_roles: Vec::new(),
    })
}

#[tokio::test]
async fn invalid_attachment_id_hex_is_rejected() {
    let deps = MockDeps::new("c1", "11111111111111111111111111111111");
    let err = download_attachment(
        &deps,
        "c1",
        "11111111111111111111111111111111",
        "not-hex",
        std::path::Path::new("/tmp/x"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, FilesError::InvalidAttachmentId(_)));
}

#[tokio::test]
async fn offer_not_found_returns_not_found() {
    let deps = MockDeps::new("c1", "11111111111111111111111111111111"); // no entries
    let err = download_attachment(
        &deps,
        "c1",
        "11111111111111111111111111111111",
        &"de".repeat(16),
        std::path::Path::new("/tmp/x"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, FilesError::NotFound(msg) if msg.contains("not found")));
}

#[tokio::test]
async fn permission_denied_blocks_download() {
    let mut deps = MockDeps::new("c1", "11111111111111111111111111111111");
    deps.permission_pass = false;
    let err = download_attachment(
        &deps,
        "c1",
        "11111111111111111111111111111111",
        &"00".repeat(16),
        std::path::Path::new("/tmp/x"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, FilesError::PermissionDenied(_)));
}

#[tokio::test]
async fn offer_with_corrupt_merkle_root_rejected() {
    // Pre-populate entries with a Message carrying an offer whose
    // chunk_hashes are all-zero but merkle_root claims something
    // else → verify_merkle_root fails.
    let aid = [3u8; 16];
    let deps = MockDeps::new("c1", "11111111111111111111111111111111")
        .with_entries(vec![build_offer_entry(aid, 2)]);
    // Tamper: make the offer's merkle_root non-zero so it doesn't
    // match the empty-hash recomputation.
    if let Some(ChannelRecordEntry::Message(msg)) = deps.channel_entries.lock().get_mut(0) {
        if let Some(o) = msg.attachment.as_mut() {
            o.merkle_root = [99u8; 32];
        }
    }
    let err = download_attachment(
        &deps,
        "c1",
        "11111111111111111111111111111111",
        &hex::encode(aid),
        std::path::Path::new("/tmp/x"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, FilesError::OfferInvalid(_)));
}
