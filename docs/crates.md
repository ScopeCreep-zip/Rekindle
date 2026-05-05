# Rust Crate Reference

Rekindle's business logic is split across **16 pure Rust crates** with zero
Tauri dependency. They form a strict tier hierarchy: lower tiers know nothing
about higher tiers, and the lowest tiers (`rekindle-types`, `rekindle-secrets`)
contain zero I/O, zero async, and zero side effects.

The `src-tauri` crate is the only place that wires these together with Tauri,
SQLite, and the Veilid runtime.

## Workspace Members

```
crates/
├── rekindle-types/                 Tier 1: shared IDs, enums, error taxonomy
├── rekindle-secrets/               Tier 2: keys, MEK, signing — sole crypto boundary
├── rekindle-codec/                 Tier 3: signed envelope build/verify, dedup
├── rekindle-records/               Tier 3: DHT record lifecycle, retry, SMPL schema
├── rekindle-utils/                 Time helpers
├── rekindle-route/                 Tier 4: private route lifecycle, peer cache
├── rekindle-gossip/                Tier 5: gossip mesh primitives
├── rekindle-governance/            Tier 6: pure CRDT merge, permissions
├── rekindle-dm/                    Tier 7: DM/group DM logic
├── rekindle-files/                 Tier 7: chunked P2P file delivery (Lost Cargo)
├── rekindle-link-preview/          Tier 7: OpenGraph fetcher (sandboxed)
├── rekindle-video/                 Tier 7: video / screen-share fragmentation
├── rekindle-sync/                  Cross-device sync (fetch, gap, history, watching)
├── rekindle-protocol/              Veilid networking, Cap'n Proto codec, DHT manager
├── rekindle-crypto/                Identity, Signal Protocol, DHT record keys
├── rekindle-game-detect/           Cross-platform game detection
├── rekindle-voice/                 Opus codec, audio I/O, jitter, mixer, transport
└── rekindle-e2e-server/            HTTP IPC bridge for Playwright E2E tests
```

Workspace-level dependencies (`Cargo.toml`):
`serde`, `serde_json`, `tokio`, `tracing`, `tracing-subscriber`, `thiserror`,
`anyhow`, `bytes`, `futures`, `parking_lot`, `capnp`, `capnpc`, `hex`, `rand`.

Workspace lints enforce `deny(warnings)`, `deny(dead_code)`,
`deny(unused-imports)`, `deny(unused-variables)`, `clippy::all = deny`,
`clippy::pedantic = warn`, plus restriction lints `dbg-macro = deny`,
`todo = deny`, `unimplemented = deny`, `undocumented-unsafe-blocks = deny`.

---

## Tier 1 — Vocabulary

### rekindle-types

Shared type definitions for the Rekindle v2.0 community system. Zero logic,
zero I/O, zero async — every other Rekindle crate depends on this.

```
src/
├── lib.rs
├── analytics.rs
├── attachment.rs
├── channel.rs
├── cross_device_sync.rs
├── error.rs
├── event.rs
├── expression.rs
├── governance.rs
├── id.rs
├── invite.rs
├── link_preview.rs
├── permissions.rs
├── presence.rs
└── search.rs
```

These are the v2.0 types for flat SMPL governance. They do **not** re-export
v1.0 types from `rekindle-protocol` — they replace them.

---

## Tier 2 — Cryptographic Boundary

### rekindle-secrets

The **sole crate** that handles raw key material. Every secret type implements
`Zeroize + ZeroizeOnDrop`. No other crate in the workspace should import
`ed25519-dalek`, `x25519-dalek`, `aes-gcm`, or `hkdf` directly.

```
src/
├── lib.rs           Re-exports ed25519_dalek for slot/keypair conversions
├── derive.rs        HKDF-SHA256 derivations (per-channel MEK, per-community pseudonym)
├── invite.rs        Invite signing keys
├── keys.rs          Ed25519 / X25519 wrappers with Zeroize
├── mek.rs           MediaEncryptionKey: AES-256-GCM symmetric key
├── rotator.rs       MEK rotation rotator (deterministic blake3-based selection)
├── sign.rs          Sign/verify helpers
└── sync_key.rs      Cross-device pairing key
```

Dependencies: `rekindle-types`, `ed25519-dalek`, `x25519-dalek`, `aes-gcm`,
`hkdf`, `blake3`, `zeroize`.

---

## Tier 3 — Wire Format and Records

### rekindle-codec

Signed envelope construction, verification, dedup, and serialization for the
gossip mesh.

```
src/
├── lib.rs
├── dedup.rs       Sliding-window dedup cache (envelope ID → seen-at)
└── envelope.rs    SignedEnvelope build/verify; CommunityEnvelope payloads
```

Dependencies: `rekindle-types`, `rekindle-secrets`.

### rekindle-records

DHT record lifecycle management for the v2.0 universal SMPL schema
(`o_cnt: 0`, 255 member slots — the "Q-pid equation"). Houses the durable
write retry queue.

```
src/
├── lib.rs
├── lifecycle.rs   open / close / republish / refresh
├── retry.rs       WriteQueueHandle — durable SMPL write retry with backoff
└── schema.rs      Universal SMPL schema constants + helpers
```

Dependencies: `rekindle-types`, `rekindle-secrets`, `veilid-core`.

---

## Tier 4 — Routing

### rekindle-route

Private route lifecycle: allocation, refresh, peer route cache.

```
src/
├── lib.rs
├── cache.rs       RouteCache — per-peer route blob + TTL eviction
├── contexts.rs    Per-purpose RoutingContext factories (priv route, safety route, unsafe)
└── lifecycle.rs   RouteLifecycle — periodic refresh, dead-route detection
```

Dependencies: `rekindle-types`, `veilid-core`.

---

## Tier 5 — Gossip Mesh

### rekindle-gossip

Transport-agnostic gossip mesh primitives. Pure logic — does not call
`app_message` itself; the integration layer plumbs the broadcast helpers
into Veilid.

```
src/
├── lib.rs
├── broadcast.rs   Generic broadcast helpers
├── dedup.rs       DedupCache (re-exported into AppState)
├── lamport.rs     Lamport clock arithmetic
├── mesh.rs        fanout_degree() — adaptive D selection (≤20 → N-1; 21–60 → 6; 61+ → 8)
└── rate_limit.rs  Sender-side token bucket
```

Dependencies: `rekindle-types`, `rekindle-codec`.

---

## Tier 6 — Governance CRDT

### rekindle-governance

**No I/O. No async. No side effects.** Takes `GovernanceEntry` variants from
all member subkeys, sorts by `(lamport, author_pseudonym)`, and applies
deterministic merge rules to produce a `GovernanceState`. Every peer running
the same merge on the same entries produces an identical result — this is
the CRDT convergence guarantee.

```
src/
├── lib.rs
├── merge.rs       merge() — the entire CRDT engine
├── permissions.rs Reader-validates: derive effective permissions for a member
├── state.rs       GovernanceState (channels, roles, members, bans, settings, …)
└── validate.rs    Entry-level validation (size, well-formedness)
```

The `proptest-regressions/merge.txt` file pins property-test seeds for the
merge function — do not delete.

Dependencies: `rekindle-types` only.

---

## Tier 7 — Self-Contained Features

### rekindle-dm

Direct messages and group DMs (architecture §27). DMs are SMPL records with
`o_cnt: 0`, exactly 2 member subkeys, and a MEK derived deterministically via
X25519 ECDH between the two identity keys (no separate key exchange round-trip).
Group DMs wrap the MEK per recipient.

Pure logic — no DHT, no Tauri. The `src-tauri/services/dm/` layer wires it
to Veilid and SQLite.

```
src/
├── lib.rs       Re-exports DmInvite, GroupDmInvite, DmMek, DmMekChain
├── error.rs     DmError
├── invite.rs    DmInvite, GroupDmInvite, GroupDmParticipant
└── mek.rs       derive_dm_mek (X25519 ECDH → HKDF), ratchet_dm_mek, DmMekChain
```

### rekindle-files

Lost Cargo: chunked Merkle-verified P2P file delivery (architecture §28.9).
Per-file FEK pattern (Signal/Matrix style), AttachmentBitmap for swarm fetch,
filesystem cache with synchronous LRU eviction, BLAKE3 chunk hashes.

```
src/
├── lib.rs
├── cache.rs       Filesystem ChunkCache (PinnedSet, LRU eviction)
├── chunker.rs     Chunk splitting with size/count limits
├── error.rs       FileError
├── manifest.rs    AttachmentManifest (chunk hashes, size, MIME)
├── pinned.rs      PinnedSet — attachments exempt from eviction
└── verify.rs      Merkle verification
```

Tier 7, pure logic, zero async, zero Tauri.

### rekindle-link-preview

Architecture §28.8 — sandboxed OpenGraph fetcher. Single public async
function `fetch_link_preview`. Hard limits: 5s timeout, 256 KB body cap,
plain text/html only, max 5 redirects, custom `User-Agent`.

### rekindle-video

Video & screen-share fragmentation/reassembly per architecture §10.6. Pure
logic — no codec FFI, no Tauri, no I/O. The actual VP9 encode/decode plugs
in via the `VideoCodec` trait at the application layer; this crate handles
only the on-the-wire framing (≤28 KB payload chunks, FEC-friendly indexing,
per-stream reassembly buffer with bounded memory).

```
src/
├── lib.rs
├── fragment.rs       fragment_frame, fragment_frame_with_fec, reconstruct_frame
└── reassembler.rs    Reassembler — per-stream buffer with bounded memory
```

---

## Cross-Cutting Integration Crates

### rekindle-protocol

Veilid networking, DHT record management, Cap'n Proto serialization, and
routing. Hosts the v1.0 `MessageEnvelope` / `MessagePayload` types still used
for 1:1 friend traffic (DM invites, friend requests, relay payloads, presence
inline updates).

```
src/
├── lib.rs            Cap'n Proto generated module includes
├── error.rs          ProtocolError
├── node.rs           RekindleNode — Veilid node lifecycle
├── routing.rs        Private route allocation, peer route import
├── peer.rs           Peer address resolution
├── capnp_codec.rs    Cap'n Proto encode/decode helpers
├── messaging/
│   ├── envelope.rs   MessageEnvelope, MessagePayload (DirectMessage,
│   │                 ChannelMessage, FriendRequest/Accept/Reject,
│   │                 ProfileKeyRotated, PresenceUpdate, Unfriended,
│   │                 RelayOffer/Withdraw/Ack/Envelope, DmInvite/Accept/
│   │                 Decline, GroupDmInvite, DmLeave, RegisterPushRelay,
│   │                 UnregisterPushRelay, WakeNotify, StatusRequest/Response)
│   ├── sender.rs     Outbound delivery via app_message
│   └── receiver.rs   Inbound dispatch
└── dht/
    ├── mod.rs        DHTManager
    ├── profile.rs    User profile record (DFLT, 8 subkeys)
    ├── presence.rs   Presence read/write
    ├── friends.rs    Friend list DHT record
    ├── conversation.rs  Per-friend encrypted conversation record
    ├── account.rs    Account record (encrypted, contact/chat/invite refs)
    ├── mailbox.rs    Mailbox DHT record (route blob inbox)
    ├── channel.rs    Channel message records
    ├── short_array.rs DHTShortArray (max 255)
    ├── log.rs        DHTLog (append-only spanning records)
    └── community/
        ├── mod.rs
        ├── envelope.rs       Community gossip envelope
        ├── manifest.rs       Manifest record helpers
        ├── member_registry.rs SMPL member registry layout
        ├── channel_record.rs  SMPL channel record layout (+ tests/)
        ├── audit_log.rs      Audit log entries
        ├── automod.rs        AutoMod rule storage
        ├── onboarding.rs     Onboarding config / welcome screen
        ├── permissions_v2.rs Permission bitmask definitions
        └── types.rs          ChannelKind, ChannelRecordKind, etc.
```

### rekindle-crypto

Cryptographic operations including Ed25519 identity, Signal Protocol session
handling, group MEK primitives, and HKDF-derived DHT record keys.

```
src/
├── lib.rs
├── error.rs
├── identity.rs        Ed25519 keypair, sign/verify, hex helpers
├── keychain.rs        Keychain trait, vault/key constants
├── dht_crypto.rs      DhtRecordKey: account/conversation key derivation +
│                      XChaCha20-Poly1305 encrypt/decrypt
├── group/
│   ├── mod.rs
│   ├── media_key.rs   MediaEncryptionKey: AES-256-GCM with generation tracking
│   └── pseudonym.rs   derive_community_pseudonym() — HKDF → unlinkable Ed25519
└── signal/
    ├── mod.rs         SignalSessionManager — X3DH + Double Ratchet
    ├── session.rs     Session establishment / encrypt / decrypt
    ├── prekeys.rs     PreKeyBundle struct
    ├── store.rs       Stronghold-backed Signal stores
    ├── memory_stores.rs / test_stores.rs  In-memory stores for testing
```

### rekindle-game-detect

Cross-platform game detection. Process scanning + JSON game database +
launcher integration.

```
src/
├── lib.rs           GameDetector public API
├── error.rs
├── scanner.rs       Scan loop with configurable interval
├── database.rs      JSON game database (process name → game info)
├── launcher.rs      Launch a game targeting a specific server
├── rich_presence.rs Server info + elapsed time tracking
└── platform/
    ├── mod.rs       list_process_names() platform abstraction
    ├── linux.rs     /proc enumeration
    ├── macos.rs     macOS process enumeration
    └── windows.rs   CreateToolhelp32Snapshot enumeration
```

### rekindle-voice

Voice chat pipeline. `cpal::Stream` is `!Send` on macOS, so capture and
playback live on dedicated OS threads and bridge to Tokio via `mpsc` channels.

```
src/
├── lib.rs            VoiceEngine — central controller
├── error.rs
├── capture.rs        cpal microphone input on dedicated thread
├── playback.rs       cpal speaker output on dedicated thread
├── codec.rs          OpusCodec (48kHz mono, VoIP mode, 32kbps, in-band FEC)
├── audio_processing.rs  RNNoise denoising + AEC3 echo cancellation + VAD
├── audio_thread.rs   Threading helpers
├── device.rs         Device enumeration / selection
├── jitter.rs         JitterBuffer — adaptive, BTreeMap by sequence
├── mixer.rs          AudioMixer — multi-participant mixing
└── transport.rs      VoiceTransport — Veilid app_message with SafetySelection::Unsafe
```

Voice packets use `SafetySelection::Unsafe` for direct UDP-like delivery,
bypassing privacy routing to minimize latency.

### rekindle-sync

Cross-device sync: fetch, gap detection, history, warming, and DHT watching.

```
src/
├── lib.rs
├── fetch.rs        Fetch missing subkeys
├── gap.rs          Gap detection across record subkeys
├── history.rs      Catch-up history fetch
├── inspect.rs      Network sequence inspection (cheaper than full fetch)
├── verify.rs       Verify fetched payloads
├── warming.rs      Record warming on first interest
└── watch.rs        watch_dht_values orchestration
```

### rekindle-utils

Time helpers (`now_ms`, `now_secs`, monotonic timestamp utilities).

```
src/
├── lib.rs
└── time.rs
```

### rekindle-e2e-server

HTTP IPC bridge that exposes Tauri commands over `localhost:3001` so
Playwright tests can drive the real Rust backend without the Tauri webview.
Used when `VITE_E2E=true`.

```
src/
└── bin/
    └── e2e_server.rs  HTTP server binary (entry point: e2e-server)
```

---

## Visual Tier Diagram

```
                    ┌────────────────┐
                    │ src-tauri      │  Tauri shell, Stronghold, SQLite
                    └───────┬────────┘
        ┌───────────────────┼─────────────────────────┐
        ▼                   ▼                         ▼
┌──────────────┐  ┌──────────────────┐   ┌──────────────────────────┐
│ rekindle-    │  │ rekindle-protocol │   │ rekindle-crypto, voice,  │
│ governance   │  │ (Veilid + capnp)  │   │ game-detect, sync, files │
│ (Tier 6)     │  │ (integration)     │   │ (T7 / cross-cutting)     │
└──────┬───────┘  └─────────┬────────┘   └────────────┬─────────────┘
       │                    │                         │
       ▼                    ▼                         ▼
              ┌──────────────────────────────┐
              │ rekindle-gossip (Tier 5)     │
              │ rekindle-route  (Tier 4)     │
              │ rekindle-codec, records (T3) │
              │ rekindle-secrets       (T2)  │
              │ rekindle-types         (T1)  │
              └──────────────────────────────┘
```
