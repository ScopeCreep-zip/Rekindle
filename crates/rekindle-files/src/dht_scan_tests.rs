use super::*;
use rekindle_codec::community::channel_record::{ChannelAttachmentCached, ChannelMessage};

fn offer_with_id(attachment_id: [u8; 16], chunk_count: u32) -> AttachmentOffer {
    AttachmentOffer {
        attachment_id,
        filename: "f.bin".into(),
        mime_type: "application/octet-stream".into(),
        total_size: 0,
        chunk_count,
        chunk_size: 0,
        merkle_root: [0u8; 32],
        chunk_hashes: Vec::new(),
        wrapped_fek: Vec::new(),
        fek_mek_generation: 0,
    }
}

fn msg_entry(attachment: Option<AttachmentOffer>) -> ChannelRecordEntry {
    ChannelRecordEntry::Message(ChannelMessage {
        sequence: 1,
        sender_pseudonym: "sender".into(),
        ciphertext: Vec::new(),
        mek_generation: 0,
        timestamp: 0,
        reply_to: None,
        revives: None,
        lamport_ts: 1,
        message_id: Some("m1".into()),
        attachment,
        flags: 0,
        mentioned_pseudonyms: Vec::new(),
        mentioned_roles: Vec::new(),
    })
}

fn full_bitmap_bytes(chunk_count: u32) -> Vec<u8> {
    AttachmentBitmap::full(chunk_count).as_bytes().to_vec()
}

fn cached_entry(
    attachment_id: [u8; 16],
    chunk_count: u32,
    author: &str,
    lamport_ts: u64,
) -> ChannelRecordEntry {
    ChannelRecordEntry::AttachmentCached(ChannelAttachmentCached {
        attachment_id,
        chunk_bitmap: full_bitmap_bytes(chunk_count),
        chunk_count,
        author_pseudonym: author.into(),
        lamport_ts,
    })
}

// ── fetch_offer_in_entries ────────────────────────────────────

#[test]
fn fetch_offer_returns_first_matching_message_attachment() {
    let target = [3u8; 16];
    let other = [9u8; 16];
    let entries = vec![
        msg_entry(None),
        msg_entry(Some(offer_with_id(other, 4))),
        msg_entry(Some(offer_with_id(target, 6))),
        msg_entry(Some(offer_with_id([7u8; 16], 2))),
    ];
    let got = fetch_offer_in_entries(&entries, target).expect("should find");
    assert_eq!(got.attachment_id, target);
    assert_eq!(got.chunk_count, 6);
}

#[test]
fn fetch_offer_returns_none_when_attachment_id_absent() {
    let entries = vec![
        msg_entry(None),
        msg_entry(Some(offer_with_id([1u8; 16], 2))),
    ];
    assert!(fetch_offer_in_entries(&entries, [99u8; 16]).is_none());
}

#[test]
fn fetch_offer_skips_non_message_entries() {
    let target = [4u8; 16];
    let entries = vec![
        cached_entry(target, 3, "peer", 1),
        msg_entry(Some(offer_with_id(target, 3))),
    ];
    let got = fetch_offer_in_entries(&entries, target).expect("should find");
    assert_eq!(got.attachment_id, target);
}

// ── discover_sources_in_entries ───────────────────────────────

#[test]
fn discover_sources_returns_one_per_author() {
    let aid = [5u8; 16];
    let entries = vec![
        cached_entry(aid, 4, "alice", 1),
        cached_entry(aid, 4, "bob", 1),
        cached_entry(aid, 4, "carol", 1),
    ];
    let sources = discover_sources_in_entries(&entries, aid, 4);
    assert_eq!(sources.len(), 3);
    let names: std::collections::HashSet<_> =
        sources.iter().map(|s| s.pseudonym.as_str()).collect();
    assert!(names.contains("alice"));
    assert!(names.contains("bob"));
    assert!(names.contains("carol"));
}

#[test]
fn discover_sources_applies_lww_by_lamport_ts() {
    let aid = [6u8; 16];
    // Build two bitmaps with different "has(0)" values to detect
    // which one was kept after the LWW dedup. lamport_ts=5 wins.
    let mut early_bitmap = AttachmentBitmap::new(2);
    early_bitmap.set(0); // only chunk 0
    let early_bytes = early_bitmap.as_bytes().to_vec();
    let mut late_bitmap = AttachmentBitmap::new(2);
    late_bitmap.set(1); // only chunk 1
    let late_bytes = late_bitmap.as_bytes().to_vec();

    let entries = vec![
        ChannelRecordEntry::AttachmentCached(ChannelAttachmentCached {
            attachment_id: aid,
            chunk_bitmap: early_bytes,
            chunk_count: 2,
            author_pseudonym: "alice".into(),
            lamport_ts: 2,
        }),
        ChannelRecordEntry::AttachmentCached(ChannelAttachmentCached {
            attachment_id: aid,
            chunk_bitmap: late_bytes,
            chunk_count: 2,
            author_pseudonym: "alice".into(),
            lamport_ts: 5,
        }),
    ];
    let sources = discover_sources_in_entries(&entries, aid, 2);
    assert_eq!(sources.len(), 1);
    let s = &sources[0];
    assert_eq!(s.pseudonym, "alice");
    // The lamport=5 entry's bitmap should be the one kept: chunk 1
    // present, chunk 0 absent.
    assert!(!s.bitmap.has(0));
    assert!(s.bitmap.has(1));
}

#[test]
fn discover_sources_rejects_mismatched_chunk_count() {
    let aid = [7u8; 16];
    let entries = vec![
        cached_entry(aid, 4, "alice", 1),
        cached_entry(aid, 8, "bob", 1), // wrong chunk_count
    ];
    let sources = discover_sources_in_entries(&entries, aid, 4);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].pseudonym, "alice");
}

#[test]
fn discover_sources_rejects_mismatched_attachment_id() {
    let target = [8u8; 16];
    let other = [9u8; 16];
    let entries = vec![
        cached_entry(target, 4, "alice", 1),
        cached_entry(other, 4, "bob", 1),
    ];
    let sources = discover_sources_in_entries(&entries, target, 4);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].pseudonym, "alice");
}

#[test]
fn discover_sources_returns_empty_when_no_matching_entries() {
    let entries = vec![msg_entry(None)];
    let sources = discover_sources_in_entries(&entries, [1u8; 16], 4);
    assert!(sources.is_empty());
}

/// The uploader writes no `AttachmentCached`: the author of the message
/// carrying the offer is a source of every chunk (plan E3.3 slice 7).
#[test]
fn the_offer_author_is_a_source_of_every_chunk() {
    let aid = [8u8; 16];
    let entries = vec![
        msg_entry(Some(offer_with_id(aid, 3))),
        msg_entry(Some(offer_with_id([1u8; 16], 3))),
    ];
    let sources = discover_sources_in_entries(&entries, aid, 3);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].pseudonym, "sender");
    assert!(sources[0].bitmap.is_complete());
    assert!(discover_sources_in_entries(&entries, aid, 4).is_empty());
}

/// Sources come back in the order their authors first appear in the
/// entries (arrival order), never in a hash order that changes from one
/// process to the next: the netsim's one seed must give one trace.
#[test]
fn sources_keep_the_arrival_order_of_their_authors() {
    let aid = [9u8; 16];
    let authors = ["hal", "bea", "zed", "ann", "cal", "eve", "dan", "gus"];
    let mut entries: Vec<_> = (authors.iter())
        .map(|a| cached_entry(aid, 3, a, 1))
        .collect();
    entries.push(cached_entry(aid, 3, "bea", 2));
    let sources = discover_sources_in_entries(&entries, aid, 3);
    let order: Vec<&str> = sources.iter().map(|s| s.pseudonym.as_str()).collect();
    assert_eq!(order, authors);
}
