//! Phase 15 — pure entry-matching helpers used by the download path.
//!
//! Architecture §28.9 — the AttachmentOffer for a file lives embedded
//! in a `ChannelEntry::Message`; possession of chunks is advertised
//! via `ChannelEntry::AttachmentCached`. After the caller scans the
//! channel SMPL record (subkeys 0..255) and decodes entries, the pure
//! helpers here turn those entries into the data the download
//! orchestrator needs: the offer + per-peer chunk bitmaps.
//!
//! No DHT calls live here — the src-tauri caller (or, eventually, the
//! `FilesDeps::scan_channel_subkeys` trait method) owns the
//! veilid-core RecordKey + RoutingContext.

use std::collections::HashMap;

use rekindle_protocol::dht::community::channel_record::ChannelRecordEntry;
use rekindle_types::attachment::{AttachmentBitmap, AttachmentOffer};

/// A peer's advertised possession of an attachment's chunks.
#[derive(Debug, Clone)]
pub struct DiscoveredSource {
    pub pseudonym: String,
    pub bitmap: AttachmentBitmap,
}

/// Find the first `Message` entry whose attachment matches the target
/// id. Returns the embedded offer, or `None` if no such entry exists
/// in the provided entry set.
#[must_use]
pub fn fetch_offer_in_entries(
    entries: &[ChannelRecordEntry],
    target_attachment_id: [u8; 16],
) -> Option<AttachmentOffer> {
    for entry in entries {
        if let ChannelRecordEntry::Message(msg) = entry {
            if let Some(offer) = &msg.attachment {
                if offer.attachment_id == target_attachment_id {
                    return Some(offer.clone());
                }
            }
        }
    }
    None
}

/// Build the per-peer source list from `AttachmentCached` entries.
/// Applies LWW per `(author_pseudonym, attachment_id)` — only the
/// highest-lamport entry per author is kept. Entries with mismatched
/// `chunk_count` or invalid bitmap-length are silently dropped.
#[must_use]
pub fn discover_sources_in_entries(
    entries: &[ChannelRecordEntry],
    attachment_id: [u8; 16],
    chunk_count: u32,
) -> Vec<DiscoveredSource> {
    let mut latest: HashMap<String, (u64, AttachmentBitmap)> = HashMap::new();
    for entry in entries {
        let ChannelRecordEntry::AttachmentCached(cached) = entry else {
            continue;
        };
        if cached.attachment_id != attachment_id {
            continue;
        }
        if cached.chunk_count != chunk_count {
            continue;
        }
        let Some(bitmap) = AttachmentBitmap::from_bytes(cached.chunk_bitmap.clone(), chunk_count)
        else {
            continue;
        };
        let prev = latest.get(&cached.author_pseudonym).map_or(0, |(l, _)| *l);
        if cached.lamport_ts >= prev {
            latest.insert(cached.author_pseudonym.clone(), (cached.lamport_ts, bitmap));
        }
    }
    latest
        .into_iter()
        .map(|(pseudonym, (_, bitmap))| DiscoveredSource { pseudonym, bitmap })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_protocol::dht::community::channel_record::{
        ChannelAttachmentCached, ChannelMessage,
    };

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
}
