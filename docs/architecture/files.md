# File Sharing — Lost Cargo

Lost Cargo is Rekindle's chunked, peer-cached file delivery system. The
name comes from Death Stranding: heavy cargo stays at the depot, and
porters who happen to be carrying it can deliver to anyone who asks.
Translated to chat: file chunks live in peer caches across the
community, and any peer that has a chunk can serve it to a peer that
needs it.

The implementation lives in **`crates/rekindle-files/`** (Tier 7, pure
logic, zero async, zero Tauri) and the Tauri-side wiring under
**`src-tauri/src/services/community/files/`**.

## Why chunked + peer-cached

Three architectural problems push toward this design:

1. **No central CDN.** Rekindle has no servers. A naive "every peer
   stores the whole file" model wastes storage and forces uploaders to
   stay online.
2. **Veilid `app_message` is small-payload.** The hard limit is ~32 KB.
   Files larger than a single message must be chunked.
3. **The privacy posture forbids gossip-of-content.** Per the Chiral
   Notification Model (see [`communities.md`](communities.md)), the
   gossip mesh carries notifications, not cargo. File ciphertext lives
   only where it is needed, fetched by peers who have an interest in it.

Lost Cargo solves all three: chunks are 28 KB (just under
`app_message`'s payload cap), they are encrypted once with a per-file
key, and any peer who downloaded a chunk can re-serve it to a peer who
asks. The original uploader can go offline as soon as the first
recipient has cached the file.

## Wire types

`AttachmentOffer` and `AttachmentBitmap` live in **`rekindle-types`**
(Tier 1) so the protocol layer can embed them without depending on
this Tier 7 crate.

```rust
struct AttachmentOffer {
    attachment_id: [u8; 16],          // fresh UUIDv4 per upload
    filename:      String,
    mime_type:     String,
    total_size:    u64,
    chunk_count:   u32,
    chunk_size:    u32,                // == 28 KB except for the last chunk
    merkle_root:   [u8; 32],          // v1: SHA-256 over concatenated chunk_hashes
    chunk_hashes:  Vec<[u8; 32]>,     // SHA-256 per chunk, in order
    wrapped_fek:   Vec<u8>,            // FEK wrapped under the channel MEK
}

struct AttachmentBitmap {
    attachment_id: [u8; 16],
    bits:          Vec<u8>,            // ceil(chunk_count / 8) bytes; bit `i` set ⇒ peer has chunk `i`
}
```

The offer is announced in the channel record (alongside the message it
attaches to). The bitmap is announced via gossip: peers periodically
broadcast which chunks they currently hold for the attachments they
care about. This lets requesters do BitTorrent-style swarm fetch — pick
the rarest chunk and ask the smallest set of peers that hold it.

## Per-file FEK (Signal/Matrix pattern)

```
plaintext chunks ─encrypt(FEK)─▶ chunk ciphertext ─store─▶ peer cache
                                                                 │
plaintext FEK ─wrap(channel MEK)─▶ wrapped_fek ─in offer─────────┘
```

A fresh File Encryption Key is generated per upload (32 bytes from
CSPRNG). All chunks are encrypted with this FEK once, and the FEK
itself is wrapped under the **current channel MEK**.

This decouples chunk storage from MEK rotation:

- **MEK rotates** (member leaves a community) → only the wrapped FEK
  in the announcement needs re-wrapping. Every peer's cached chunk
  ciphertext stays valid.
- Without per-file FEK, every chunk would have to be re-encrypted
  on every MEK rotation — making peer caches useless.

This pattern is borrowed from Signal (per-attachment AES key) and
Matrix (per-event content key).

## Chunking and Merkle root

```rust
const CHUNK_SIZE_BYTES: usize    = 28 * 1024;
const MAX_FILE_SIZE_BYTES: u64   = 28 * 1024 * 1000;  // ≈ 28 MB (v1 cap)
```

The 28 KB chunk size matches Veilid's `app_message` payload cap minus
overhead. The 28 MB file-size cap exists because v1's flat-list Merkle
root stores `chunk_hashes` (32 B × 1000 chunks = 32 KB exact) inside a
single SMPL subkey, which Veilid limits to 32 KB. v2 will switch to a
true [BEP-52](https://www.bittorrent.org/beps/bep_0052.html) binary
Merkle tree with sibling proofs to lift the cap.

```rust
merkle_root = SHA256(chunk_hashes[0] ‖ chunk_hashes[1] ‖ … ‖ chunk_hashes[N-1])
chunk_hashes[i] = SHA256(plaintext_chunk_i)
```

Verification:

- **Per-chunk:** when a peer delivers chunk `i`, decrypt with FEK, then
  compare `SHA256(plaintext)` against `chunk_hashes[i]`. Mismatch ⇒
  drop the peer's contribution and ask elsewhere (the bitmap will tell
  you who else holds it).
- **Per-offer:** when an `AttachmentOffer` first arrives over the wire,
  recompute the flat-list Merkle root over the announced
  `chunk_hashes` and compare against the announced `merkle_root`. This
  protects against an attacker forging the announcement — even though
  the announcement is signed, recomputing prevents a class of
  serialization-confusion attacks.

## Filesystem cache

```
<app_data>/file_cache/<community_id>/<aa>/<full_attachment_hex>/<chunk_index>.bin
                                       │
                                       └── git-style 2-char fanout (256-way)
```

- **Path format** is git-style: first 2 hex chars of the attachment ID
  fan out the cache directory, then the full attachment hex, then one
  file per cached chunk.
- **Per-chunk** files (not one big concatenated blob). This lets
  eviction work at chunk granularity and makes resumed downloads
  trivial.
- **`.meta` sidecar** stores chunk size and the bitmap of locally-held
  chunks for quick listing without a full directory walk.

The cache is namespaced per community (`<app_data>/file_cache/<community_id>`)
so that leaving a community cleanly removes all its cached cargo.

## LRU eviction with pinned-skip

```
ChunkCache {
    config:      CacheConfig { root_dir, byte_budget: 1 GiB default },
    lru:         LruCache<(attachment_id, chunk_index), bytes_on_disk>,
    total_bytes: u64,
}
```

Eviction is **synchronous after every `insert`** — no background
sweeper. After writing a new chunk:

1. While `total_bytes > byte_budget`:
   - Pop the LRU entry.
   - If the attachment is pinned (in the `PinnedSet`), skip and try the
     next one.
   - Otherwise, delete the file and decrement `total_bytes`.
2. Empty attachment directories are GC'd lazily on next `open()`.

The 1 GiB default is per-community. Pinning is governed by
`GovernanceEntry::AttachmentPinned` — admins can pin important files
(community resources, server-config sheets, etc.) so they survive
eviction even on members with tight budgets.

On startup, `ChunkCache::open()` walks the cache directory and
registers every existing chunk as a least-recently-used entry. The LRU
ordering is recovered from filesystem mtime where available, but a
crash that loses the in-memory ordering only loses ordering — the
chunks themselves are durable.

## Swarm fetch protocol

```
Requester                                                   Holders
   │                                                         │
   │── gossip: AttachmentBitmap interest ───────────────▶    │
   │                                                         │
   │◀── gossip: AttachmentBitmap (I have chunks 3,7,9) ──    │
   │◀── gossip: AttachmentBitmap (I have chunks 0,1,2,3) ─   │
   │                                                         │
   │ rarest-first scheduler picks chunk 7 from peer A,       │
   │ chunk 0 from peer B, chunk 3 from either, …             │
   │                                                         │
   │── app_call: RequestChunk(att_id, idx=7) ────────▶ peer A│
   │◀── app_call reply: <ciphertext_chunk_7> ──────────────  │
   │ verify_chunk → store → update local bitmap → broadcast  │
   │                                                         │
```

Chunks travel over `app_call` (acknowledged delivery) rather than
`app_message`, so the requester knows when a chunk arrived versus when
the peer is offline. A failed `app_call` with `TryAgain` makes the
requester reroute to a different holder rather than retry the same
peer.

The architecture spec calls this the *Lost Cargo locker* model — the
cargo (chunks) sit at depots (peer caches), porters carry notifications
(bitmaps), and any depot can fulfil any request.

## Pinning

```rust
GovernanceEntry::AttachmentPinned {
    attachment_id: Uuid,
    pinned: bool,
    lamport: u64,
}
```

Pinning is a governance entry — admins with `MANAGE_MESSAGES` (or
`PIN_MESSAGES` for ordinary pins) write it. Members merge the resulting
`PinnedSet` from the CRDT state and treat the pinned attachments as
exempt from local LRU eviction.

This gives communities a way to keep important cargo around without
relying on every member's individual cache budget.

## Open work

- **BEP-52 binary Merkle tree** for files >28 MB.
- **Resumable uploads** that survive sender restart by checkpointing
  chunk progress to local SQLite.
- **Frontend UI polish** — drag-and-drop, progress display, cancel,
  attachment preview.
- **Lost Cargo for non-channel contexts** (DM file send) — currently
  routes through the same crate but with a 2-member SMPL record.

Tracked in [`../roadmap.md`](../roadmap.md).

## Where to look

| Concern | File |
|---------|------|
| Chunker + Merkle root | `crates/rekindle-files/src/chunker.rs` |
| Manifest validation | `crates/rekindle-files/src/manifest.rs` |
| Verification (per-chunk + per-offer) | `crates/rekindle-files/src/verify.rs` |
| Filesystem cache + LRU | `crates/rekindle-files/src/cache.rs` |
| Pinned-attachment set | `crates/rekindle-files/src/pinned.rs` |
| `AttachmentOffer` / `AttachmentBitmap` types | `crates/rekindle-types/src/attachment.rs` |
| Tauri-side service wiring | `src-tauri/src/services/community/files/` (and adjacent modules) |
| IPC commands | `src-tauri/src/commands/community/files.rs` |
