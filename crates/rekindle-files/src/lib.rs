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
pub mod error;
pub mod manifest;
pub mod pinned;
pub mod verify;

pub use cache::{community_cache_root, CacheConfig, ChunkCache};
pub use chunker::{merkle_root_of, ChunkedFile, Chunker, CHUNK_SIZE_BYTES, MAX_FILE_SIZE_BYTES};
pub use error::FilesError;
pub use manifest::validate_offer;
pub use pinned::PinnedSet;
pub use rekindle_types::attachment::{AttachmentBitmap, AttachmentOffer};
pub use verify::{verify_chunk, verify_merkle_root};
