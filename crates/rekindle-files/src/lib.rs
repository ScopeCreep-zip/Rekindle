//! Lost Cargo file sharing for Rekindle v2.0.
//!
//! Implements the chunked Merkle-verified P2P file delivery described in
//! `.claude/docs/rekindle-communities-architecture.md` §28.9.
//!
//! Tier 7 — pure logic, zero async, zero Tauri. The Tauri shell wires this
//! crate to the protocol layer (channel records, control payloads) and to
//! the Veilid `app_call` dispatcher.
//!
//! # Design departures from a literal reading of §28.9
//!
//! These resolve spec silences. See the plan in
//! `.claude/plans/modular-brewing-blanket.md` §1.J for the rationale.
//!
//! - **Per-file FEK** (Signal/Matrix pattern): chunks are encrypted once with
//!   a per-file File Encryption Key; the FEK is wrapped under the channel
//!   MEK in the `AttachmentOffer`. Blob chunks survive MEK rotation — only
//!   the wrapped FEK in retained announcements needs re-wrapping.
//! - **AttachmentBitmap**: peers advertise *which* chunks they hold, not just
//!   how many — enables BitTorrent-style swarm fetch where each peer serves
//!   a disjoint chunk range.
//! - **Filesystem cache** (not SQLite BLOB): chunks live at
//!   `<root_dir>/<community_id>/<aa>/<full_attachment_hex>/<chunk_index>.bin`
//!   with a `.meta` sidecar. Git-style 2-char fanout.
//! - **Insert-time eviction**: LRU eviction runs synchronously after every
//!   `cache.insert()` until `total_bytes ≤ budget`; pinned attachments are
//!   skipped. No background sweeper.
//! - **Flat-list Merkle root** (v1): `merkle_root = SHA256(chunk_hashes
//!   concatenated)`. v2 will switch to a true BEP-52 binary tree with
//!   sibling proofs to support files >25 MB whose chunk-hash list would
//!   otherwise exceed the 32 KB SMPL subkey limit.

pub mod cache;
pub mod chunker;
pub mod deps;
pub mod dht_scan;
pub mod download;
pub mod error;
pub mod expression_fetch;
pub mod expressions;
pub mod fek;
pub mod manifest;
pub mod pinned;
pub mod serve;
pub mod upload;
pub mod verify;

#[cfg(test)]
mod test_mock;

pub use cache::{community_cache_root, CacheConfig, ChunkCache};
pub use chunker::{merkle_root_of, ChunkedFile, Chunker, CHUNK_SIZE_BYTES, MAX_FILE_SIZE_BYTES};
pub use deps::{read_bitmap_for, FilesDeps, FilesEvent, SharedFilesDeps};
pub use dht_scan::{discover_sources_in_entries, fetch_offer_in_entries, DiscoveredSource};
pub use download::download_attachment;
pub use error::FilesError;
pub use expression_fetch::eager_fetch_missing;
pub use expressions::{read_expression_bytes, upload_expression_to_cache};
pub use fek::unwrap_fek_for_offer;
pub use manifest::validate_offer;
pub use pinned::{set_attachment_pinned, sync_pinned_from_governance, PinnedSet};
pub use rekindle_types::attachment::{AttachmentBitmap, AttachmentOffer};
pub use serve::serve_attachment_request;
pub use upload::{
    guess_mime_type, send_voice_message_bytes, upload_bytes_as_attachment, upload_file,
    AttachmentRecordJson,
};
pub use verify::{verify_chunk, verify_merkle_root};
