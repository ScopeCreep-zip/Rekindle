//! The download asks every advertising source in arrival order (§28.9):
//! a source that is offline or never answers is recorded and the next one
//! is asked; the attempt fails only when no source completes it.

use rekindle_codec::community::channel_record::{
    ChannelAttachmentCached, ChannelMessage, ChannelRecordEntry,
};
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_types::attachment::{AttachmentBitmap, AttachmentOffer};

use crate::chunker::{Chunker, CHUNK_SIZE_BYTES};
use crate::error::FilesError;
use crate::test_mock::MockDeps;

const CHANNEL: &str = "11111111111111111111111111111111";
const MEK_BYTES: [u8; 32] = [0x42; 32];
const FEK_BYTES: [u8; 32] = [0x17; 32];
const AID: [u8; 16] = [0xA5; 16];

/// A real two-chunk file: its bytes, its offer and its FEK-encrypted
/// chunks as a source serves them.
fn real_file() -> (Vec<u8>, AttachmentOffer, Vec<Vec<u8>>) {
    let bytes: Vec<u8> = (0..CHUNK_SIZE_BYTES + 100)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    let chunked = Chunker::chunk(&bytes, AID).unwrap();
    let mek = MediaEncryptionKey::from_bytes(MEK_BYTES, 0);
    let fek = MediaEncryptionKey::from_bytes(FEK_BYTES, 0);
    let offer = AttachmentOffer {
        attachment_id: AID,
        filename: "f.bin".into(),
        mime_type: "application/octet-stream".into(),
        total_size: bytes.len() as u64,
        chunk_count: u32::try_from(chunked.chunks.len()).unwrap(),
        chunk_size: u32::try_from(CHUNK_SIZE_BYTES).unwrap(),
        merkle_root: chunked.merkle_root,
        chunk_hashes: chunked.chunk_hashes.clone(),
        wrapped_fek: mek.encrypt(&FEK_BYTES).unwrap(),
        fek_mek_generation: 0,
    };
    let served = chunked
        .chunks
        .iter()
        .map(|c| fek.encrypt(c).unwrap())
        .collect();
    (bytes, offer, served)
}

/// The offer posted by `uploader` (a full source) and `cached`'s
/// AttachmentCached advertisement after it, in arrival order.
fn entries(offer: &AttachmentOffer, uploader: &str, cached: &str) -> Vec<ChannelRecordEntry> {
    vec![
        ChannelRecordEntry::Message(ChannelMessage {
            sequence: 1,
            sender_pseudonym: uploader.into(),
            ciphertext: Vec::new(),
            mek_generation: 0,
            timestamp: 0,
            reply_to: None,
            revives: None,
            lamport_ts: 1,
            message_id: Some("m1".into()),
            attachment: Some(offer.clone()),
            flags: 0,
            mentioned_pseudonyms: Vec::new(),
            mentioned_roles: Vec::new(),
        }),
        ChannelRecordEntry::AttachmentCached(ChannelAttachmentCached {
            attachment_id: AID,
            chunk_bitmap: AttachmentBitmap::full(offer.chunk_count)
                .as_bytes()
                .to_vec(),
            chunk_count: offer.chunk_count,
            author_pseudonym: cached.into(),
            lamport_ts: 2,
        }),
    ]
}

fn reply(served: &[Vec<u8>], indices: &[u32]) -> Vec<u8> {
    let chunks = indices
        .iter()
        .map(|&i| ControlPayload::AttachmentChunk {
            attachment_id: AID,
            chunk_index: i,
            data: served[i as usize].clone(),
            plaintext_hash: [0u8; 32],
        })
        .collect();
    rekindle_codec::capnp_envelope::encode_community_envelope(&CommunityEnvelope::Control(
        ControlPayload::MultiAttachmentChunk { chunks },
    ))
    .unwrap()
}

fn silent() -> Result<Vec<u8>, FilesError> {
    Err(FilesError::Transport("app_call timed out".into()))
}

fn deps(offer: &AttachmentOffer, replies: Vec<Result<Vec<u8>, FilesError>>) -> MockDeps {
    let deps = MockDeps::new("c1", CHANNEL)
        .with_mek(0, MEK_BYTES)
        .with_peer("silent", vec![1])
        .with_peer("server", vec![2])
        .with_entries(entries(offer, "silent", "server"));
    deps.replies.lock().queue.extend(replies);
    deps
}

#[tokio::test]
async fn a_silent_first_source_is_skipped_and_the_next_serves_the_file() {
    let (bytes, offer, served) = real_file();
    let deps = deps(&offer, vec![silent(), Ok(reply(&served, &[0, 1]))]);
    let fetched = super::fetch_attachment_to_cache(&deps, "c1", CHANNEL, &hex::encode(AID))
        .await
        .expect("the second source completes the download");
    let assembled = super::assemble_attachment(&deps, "c1", &fetched).unwrap();
    assert_eq!(assembled, bytes);
    assert_eq!(deps.calls.lock().app_calls.len(), 2, "both sources asked");
    assert_eq!(
        deps.calls.lock().posts_queued.len(),
        1,
        "possession advertised"
    );
}

#[tokio::test]
async fn an_offline_first_source_is_skipped_and_the_next_serves_the_file() {
    let (bytes, offer, served) = real_file();
    let mut deps = deps(&offer, vec![Ok(reply(&served, &[0, 1]))]);
    deps.peers.remove("silent");
    let fetched = super::fetch_attachment_to_cache(&deps, "c1", CHANNEL, &hex::encode(AID))
        .await
        .expect("the online source completes the download");
    assert_eq!(
        super::assemble_attachment(&deps, "c1", &fetched).unwrap(),
        bytes
    );
}

#[tokio::test]
async fn chunks_verified_from_two_sources_assemble() {
    let (bytes, offer, served) = real_file();
    // The first source serves chunk 0, then goes silent; the second
    // serves chunk 1.
    let deps = deps(
        &offer,
        vec![Ok(reply(&served, &[0])), silent(), Ok(reply(&served, &[1]))],
    );
    let fetched = super::fetch_attachment_to_cache(&deps, "c1", CHANNEL, &hex::encode(AID))
        .await
        .expect("two sources complete the download");
    assert_eq!(
        super::assemble_attachment(&deps, "c1", &fetched).unwrap(),
        bytes
    );
    assert_eq!(deps.calls.lock().app_calls.len(), 3);
}

#[tokio::test]
async fn a_tampered_chunk_is_dropped_and_another_source_supplies_it() {
    let (bytes, offer, mut served) = real_file();
    let good = served.clone();
    served[1][0] ^= 0xFF;
    let deps = deps(
        &offer,
        vec![Ok(reply(&served, &[0, 1])), Ok(reply(&good, &[1]))],
    );
    let fetched = super::fetch_attachment_to_cache(&deps, "c1", CHANNEL, &hex::encode(AID))
        .await
        .expect("the good copy of chunk 1 completes the download");
    assert_eq!(
        super::assemble_attachment(&deps, "c1", &fetched).unwrap(),
        bytes
    );
}

#[tokio::test]
async fn the_attempt_fails_cleanly_when_every_source_fails() {
    let (_, offer, _) = real_file();
    let deps = deps(&offer, vec![silent(), silent()]);
    let Err(err) = super::fetch_attachment_to_cache(&deps, "c1", CHANNEL, &hex::encode(AID)).await
    else {
        panic!("the attempt must fail when every source fails");
    };
    assert!(
        matches!(err, FilesError::DownloadIncomplete { have: 0, total: 2 }),
        "{err:?}"
    );
    assert_eq!(deps.calls.lock().app_calls.len(), 2, "every source asked");
    assert!(
        deps.calls.lock().posts_queued.is_empty(),
        "nothing advertised"
    );
}
