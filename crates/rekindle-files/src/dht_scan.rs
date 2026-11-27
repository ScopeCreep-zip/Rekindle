//! Phase 15 — pure entry-matching helpers used by the download path.
//!
//! Architecture §28.9 — the AttachmentOffer for a file lives embedded
//! in a `ChannelEntry::Message`; possession of chunks is advertised
//! via `ChannelEntry::AttachmentCached`. Given the channel's entries
//! (`FilesDeps::channel_entries`: the members' outbox entries of the
//! channel), the pure helpers here turn them into
//! the data the download orchestrator needs: the offer + per-peer chunk
//! bitmaps. No DHT calls live here.

use std::collections::HashMap;

use rekindle_codec::community::channel_record::ChannelRecordEntry;
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

/// Build the per-peer source list: the author of the message carrying the
/// offer holds every chunk (it sealed the offer over its cached chunks, so
/// it writes no `AttachmentCached`, plan E3.3 slice 7), and each
/// `AttachmentCached` entry's author holds its bitmap. Applies LWW per
/// `(author_pseudonym, attachment_id)` — only the highest-lamport entry per
/// author is kept. Entries with mismatched `chunk_count` or invalid
/// bitmap-length are silently dropped.
#[must_use]
pub fn discover_sources_in_entries(
    entries: &[ChannelRecordEntry],
    attachment_id: [u8; 16],
    chunk_count: u32,
) -> Vec<DiscoveredSource> {
    // Each author's newest possession, kept in the order its author first
    // appears in `entries` (the arrival order the orchestrator walks), so
    // one channel state gives one source order on every run.
    let mut latest: Vec<(String, u64, AttachmentBitmap)> = Vec::new();
    let mut slot_of: HashMap<String, usize> = HashMap::new();
    let mut keep = |author: &str, lamport: u64, bitmap: AttachmentBitmap| match slot_of.get(author)
    {
        Some(&slot) if lamport >= latest[slot].1 => {
            latest[slot] = (author.to_owned(), lamport, bitmap);
        }
        Some(_) => {}
        None => {
            slot_of.insert(author.to_owned(), latest.len());
            latest.push((author.to_owned(), lamport, bitmap));
        }
    };
    for entry in entries {
        let cached = match entry {
            ChannelRecordEntry::Message(msg)
                if msg.attachment.as_ref().is_some_and(|offer| {
                    offer.attachment_id == attachment_id && offer.chunk_count == chunk_count
                }) =>
            {
                keep(
                    &msg.sender_pseudonym,
                    u64::MAX,
                    AttachmentBitmap::full(chunk_count),
                );
                continue;
            }
            ChannelRecordEntry::AttachmentCached(cached) => cached,
            _ => continue,
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
        keep(&cached.author_pseudonym, cached.lamport_ts, bitmap);
    }
    latest
        .into_iter()
        .map(|(pseudonym, _, bitmap)| DiscoveredSource { pseudonym, bitmap })
        .collect()
}

#[cfg(test)]
#[path = "dht_scan_tests.rs"]
mod tests;
