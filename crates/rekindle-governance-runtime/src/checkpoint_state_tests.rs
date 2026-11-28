use std::cell::RefCell;

use rekindle_codec::checkpoint_state::{CheckpointStateResponse, SNAPSHOT_CHUNK_MAX};
use rekindle_governance::checkpoint::state_root_of;

use super::{chunk_at, first_matching};
use crate::GovernanceRuntimeError;

/// A snapshot of three chunks, the last one short.
fn snapshot(fill: u8) -> Vec<u8> {
    vec![fill; 2 * SNAPSHOT_CHUNK_MAX + 1234]
}

fn size(snapshot: &[u8]) -> u32 {
    u32::try_from(snapshot.len()).expect("a test snapshot fits")
}

fn serve(
    snapshot: &[u8],
    offset: u32,
) -> std::future::Ready<Result<CheckpointStateResponse, GovernanceRuntimeError>> {
    std::future::ready(chunk_at(snapshot, offset))
}

#[test]
fn chunks_cover_the_snapshot_and_stop_at_its_end() {
    let s = snapshot(1);
    let first = chunk_at(&s, 0).unwrap();
    assert_eq!(first.total as usize, s.len());
    assert_eq!(first.bytes.len(), SNAPSHOT_CHUNK_MAX);
    let offset = u32::try_from(2 * SNAPSHOT_CHUNK_MAX).unwrap();
    assert_eq!(chunk_at(&s, offset).unwrap().bytes.len(), 1234);
    assert!(chunk_at(&s, u32::try_from(s.len()).unwrap()).is_err());
    assert!(
        chunk_at(&[], 0).unwrap().bytes.is_empty(),
        "an empty snapshot is one empty chunk"
    );
}

/// Plan E3.2 S7c.3: the reader checks the reassembled bytes against
/// `stateRoot` before decoding and asks the next ring peer on a mismatch.
/// The nearest peer serves well-formed chunks of the wrong state; its bytes
/// are refused and the second peer's, which match, are kept.
#[tokio::test]
async fn a_state_snapshot_that_misses_the_state_root_is_refused_and_another_peer_asked() {
    let good = snapshot(1);
    let bad = snapshot(2);
    let root = state_root_of(&good);
    let asked = RefCell::new(Vec::new());
    let got = first_matching(3, (&root, size(&good)), |peer, offset| {
        asked.borrow_mut().push((peer, offset));
        serve(if peer == 0 { &bad } else { &good }, offset)
    })
    .await;
    assert_eq!(got.as_deref(), Some(good.as_slice()));
    let peers: Vec<usize> = asked.borrow().iter().map(|(p, _)| *p).collect();
    assert_eq!(
        peers,
        [0, 0, 0, 1, 1, 1],
        "peer 0 refused, peer 1 asked, peer 2 never"
    );
}

/// No peer matches: `None`, so the caller walks the pre-cut ops.
#[tokio::test]
async fn no_matching_peer_is_none() {
    let bad = snapshot(2);
    let root = state_root_of(&snapshot(1));
    assert!(
        first_matching(2, (&root, size(&bad)), |_, offset| serve(&bad, offset))
            .await
            .is_none()
    );
}

/// A peer that drips short chunks, or changes its total, is not
/// reassembled: every chunk but the last is a full one.
#[tokio::test]
async fn a_short_or_inconsistent_chunk_ends_that_peer() {
    let good = snapshot(1);
    let root = state_root_of(&good);
    let short = |_: usize, offset: u32| {
        let mut c = chunk_at(&good, offset).unwrap();
        c.bytes.truncate(1);
        std::future::ready(Ok(c))
    };
    assert!(first_matching(1, (&root, size(&good)), short)
        .await
        .is_none());
    let shifting = |_: usize, offset: u32| {
        let mut c = chunk_at(&good, offset).unwrap();
        c.total += offset;
        std::future::ready(Ok(c))
    };
    assert!(first_matching(1, (&root, size(&good)), shifting)
        .await
        .is_none());
}

/// Plan E3.2 S7c.4a: a peer claiming a total other than the endorsed
/// `stateSize` (here `u32::MAX`) is dropped at its first chunk, never asked
/// for a second.
#[tokio::test]
async fn a_total_other_than_the_endorsed_state_size_is_refused_at_the_first_chunk() {
    let good = snapshot(1);
    let root = state_root_of(&good);
    let asked = RefCell::new(0);
    let inflated = |_: usize, offset: u32| {
        *asked.borrow_mut() += 1;
        let mut c = chunk_at(&good, offset).unwrap();
        c.total = u32::MAX;
        std::future::ready(Ok(c))
    };
    assert!(first_matching(1, (&root, size(&good)), inflated)
        .await
        .is_none());
    assert_eq!(*asked.borrow(), 1);
}
