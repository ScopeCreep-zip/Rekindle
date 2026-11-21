//! The warm set's bound (plan E3.2 S7): the latest checkpoint's directory
//! pages plus our own op pages beyond its cut, across two checkpoints.

use rekindle_codec::capnp_codec::chain::PagePointer;
use rekindle_types::governance::{OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use super::SealedPages;

fn head(author: &PseudonymKey, seq: u64) -> OpRef {
    OpRef {
        op_hash: OpHash(*blake3::hash(&seq.to_le_bytes()).as_bytes()),
        author: author.clone(),
        author_record: "M".into(),
        author_seq: seq,
    }
}

fn page(first: u64, last: u64) -> PagePointer {
    PagePointer {
        page_key: format!("own{first}"),
        first_seq: first,
        last_seq: last,
    }
}

fn keys(pages: &SealedPages) -> Vec<String> {
    pages.records().into_iter().map(|r| r.key).collect()
}

/// Two checkpoints: each replaces the directory (ceil(n / 128) pages) and
/// releases our pages its cut folds; a proposal below the held height is
/// not taken; every record is sealed.
#[test]
fn the_warm_set_is_the_latest_directory_and_own_pages_beyond_its_cut() {
    let me = PseudonymKey([1; 32]);
    let other = PseudonymKey([2; 32]);
    let mut pages = SealedPages::default();
    for (first, last) in [(0, 3), (4, 6), (7, 9)] {
        pages.own_page(&page(first, last));
    }
    // Height 1: 200 members = 2 pages; our head in the cut is seq 5.
    let dir1 = vec!["d1a".to_string(), "d1b".to_string()];
    pages.checkpoint(1, &dir1, &[head(&me, 5), head(&other, 40)], Some(&me));
    assert_eq!(keys(&pages), ["d1a", "d1b", "own4", "own7"]);
    assert!(pages.records().iter().all(|r| r.immutable));

    // Height 2: 300 members = 3 pages; our head is seq 9: all ours folded.
    pages.own_page(&page(10, 12));
    let dir2: Vec<String> = ["d2a", "d2b", "d2c"].map(String::from).to_vec();
    pages.checkpoint(2, &dir2, &[head(&me, 9)], Some(&me));
    assert_eq!(keys(&pages), ["d2a", "d2b", "d2c", "own10"]);

    // A height-1 checkpoint seen late changes nothing.
    pages.checkpoint(1, &dir1, &[head(&me, 12)], Some(&me));
    assert_eq!(keys(&pages), ["d2a", "d2b", "d2c", "own10"]);
}

/// A cut without a head of ours folds none of our pages.
#[test]
fn a_cut_without_our_head_keeps_our_pages() {
    let me = PseudonymKey([1; 32]);
    let mut pages = SealedPages::default();
    pages.own_page(&page(0, 2));
    pages.checkpoint(
        1,
        &["d".into()],
        &[head(&PseudonymKey([3; 32]), 8)],
        Some(&me),
    );
    assert_eq!(keys(&pages), ["d", "own0"]);
}
