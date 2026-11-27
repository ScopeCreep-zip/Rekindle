use super::*;
use crate::cache::{CacheConfig, ChunkCache};
use crate::pinned::PinnedSet;
use tempfile::TempDir;
use zeroize::Zeroizing;

fn open_cache(dir: &TempDir) -> ChunkCache {
    ChunkCache::open(CacheConfig {
        root_dir: dir.path().to_path_buf(),
        byte_budget: 1024 * 1024,
        name_key: Zeroizing::new([3u8; 32]),
    })
    .unwrap()
}

fn parse_reply(bytes: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let env =
        rekindle_codec::capnp_envelope::decode_community_envelope(bytes).expect("decode reply");
    let Some(ControlPayload::MultiAttachmentChunk { chunks }) = env.into_control() else {
        panic!("unexpected reply variant");
    };
    chunks
        .into_iter()
        .filter_map(|p| match p {
            ControlPayload::AttachmentChunk {
                chunk_index, data, ..
            } => Some((chunk_index, data)),
            _ => None,
        })
        .collect()
}

#[test]
fn returns_none_when_cache_has_no_matching_chunks() {
    let temp = TempDir::new().unwrap();
    let mut cache = open_cache(&temp);
    let aid = [7u8; 16];
    let reply = serve_attachment_request(&mut cache, aid, &[0, 1, 2]);
    assert!(reply.is_none(), "empty cache should yield no reply");
}

#[test]
fn returns_subset_when_cache_holds_some_chunks() {
    let temp = TempDir::new().unwrap();
    let mut cache = open_cache(&temp);
    let aid = [9u8; 16];
    let attachment_uuid = Uuid::from_bytes(aid);
    let pinned = PinnedSet::new();
    cache
        .insert(attachment_uuid, 0, b"chunk0", &pinned)
        .unwrap();
    cache
        .insert(attachment_uuid, 2, b"chunk2", &pinned)
        .unwrap();

    let reply = serve_attachment_request(&mut cache, aid, &[0, 1, 2, 3])
        .expect("should reply with held chunks");
    let parsed = parse_reply(&reply);
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0], (0, b"chunk0".to_vec()));
    assert_eq!(parsed[1], (2, b"chunk2".to_vec()));
}

#[test]
fn returns_full_set_when_all_chunks_present() {
    let temp = TempDir::new().unwrap();
    let mut cache = open_cache(&temp);
    let aid = [42u8; 16];
    let attachment_uuid = Uuid::from_bytes(aid);
    let pinned = PinnedSet::new();
    for i in 0..3u32 {
        cache
            .insert(attachment_uuid, i, format!("chunk{i}").as_bytes(), &pinned)
            .unwrap();
    }

    let reply = serve_attachment_request(&mut cache, aid, &[0, 1, 2]).expect("full reply");
    let parsed = parse_reply(&reply);
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0].0, 0);
    assert_eq!(parsed[1].0, 1);
    assert_eq!(parsed[2].0, 2);
}

#[test]
fn returns_none_when_requested_attachment_id_differs() {
    let temp = TempDir::new().unwrap();
    let mut cache = open_cache(&temp);
    let pinned = PinnedSet::new();
    let stored_aid = [1u8; 16];
    let stored_uuid = Uuid::from_bytes(stored_aid);
    cache.insert(stored_uuid, 0, b"data", &pinned).unwrap();

    let other_aid = [2u8; 16];
    let reply = serve_attachment_request(&mut cache, other_aid, &[0, 1]);
    assert!(reply.is_none());
}

/// Full chunks do not all fit one app-call answer: the reply carries
/// the chunks that fit under Veilid's message limit, in request order.
#[test]
fn a_reply_carries_only_the_chunks_that_fit_one_app_call() {
    let temp = TempDir::new().unwrap();
    let mut cache = open_cache(&temp);
    let aid = [5u8; 16];
    let attachment_uuid = Uuid::from_bytes(aid);
    let pinned = PinnedSet::new();
    let full = vec![7u8; crate::CHUNK_SIZE_BYTES + 28];
    for i in 0..3u32 {
        cache.insert(attachment_uuid, i, &full, &pinned).unwrap();
    }

    let reply = serve_attachment_request(&mut cache, aid, &[0, 1, 2]).expect("a reply");
    assert!(reply.len() <= MAX_APP_MESSAGE_LEN, "{} B", reply.len());
    let parsed = parse_reply(&reply);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0], (0, full));
}
