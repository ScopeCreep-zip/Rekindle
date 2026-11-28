use super::*;
use crate::test_mock::MockDeps;

fn offer_with_id(eid: [u8; 16], chunk_count: u32) -> AttachmentOffer {
    AttachmentOffer {
        attachment_id: eid,
        filename: "expr.png".into(),
        mime_type: "image/png".into(),
        total_size: 0,
        chunk_count,
        chunk_size: 0,
        merkle_root: [0u8; 32],
        chunk_hashes: vec![[0u8; 32]; chunk_count as usize],
        wrapped_fek: Vec::new(),
        fek_mek_generation: 0,
    }
}

#[tokio::test]
async fn no_expressions_is_no_op() {
    let deps = MockDeps::new("c1", "11111111111111111111111111111111");
    eager_fetch_missing(&deps, "c1").await;
    assert!(
        deps.calls.lock().events.is_empty(),
        "no events when nothing to fetch"
    );
    assert!(
        deps.calls.lock().app_calls.is_empty(),
        "no app_calls without candidates"
    );
}

#[tokio::test]
async fn no_online_peers_skips_fetch() {
    let mut deps = MockDeps::new("c1", "11111111111111111111111111111111").with_mek(1, [0u8; 32]);
    // One incomplete expression but no online peers.
    deps.expressions = vec![([4u8; 16], offer_with_id([4u8; 16], 2))];
    eager_fetch_missing(&deps, "c1").await;
    assert!(deps.calls.lock().app_calls.is_empty(), "skip without peers");
    assert!(deps.calls.lock().events.is_empty(), "no events emitted");
}

#[tokio::test]
async fn fully_cached_expression_is_skipped_from_candidates() {
    // Expression is already fully cached locally → no peer
    // fetch attempted. The mock starts with an empty cache, so to
    // simulate "fully cached" we insert all chunks for the
    // expression id first.
    let eid = [5u8; 16];
    let deps = MockDeps::new("c1", "11111111111111111111111111111111")
        .with_mek(1, [0u8; 32])
        .with_peer("peer-1", vec![1, 2, 3]);
    // Pre-load full bitmap into the cache.
    let uuid = Uuid::from_bytes(eid);
    {
        let mut cache = deps.cache.lock();
        cache.insert(uuid, 0, b"chunk0", &deps.pinned).unwrap();
        cache.insert(uuid, 1, b"chunk1", &deps.pinned).unwrap();
    }
    let mut deps = deps;
    deps.expressions = vec![(eid, offer_with_id(eid, 2))];
    eager_fetch_missing(&deps, "c1").await;
    assert!(
        deps.calls.lock().app_calls.is_empty(),
        "no fetch attempted for fully-cached expression"
    );
}
