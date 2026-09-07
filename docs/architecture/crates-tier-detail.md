# Crate Detail by Tier

Per-crate descriptions for every workspace member. The inventory matrix
and tier diagram live in [`crates.md`](crates.md); this file is the
narrative companion. Read top-to-bottom for an introduction to the stack,
or jump to a specific crate via the section anchors.

## Tier 1 — Vocabulary

### rekindle-types

Shared type definitions for the Rekindle v2.0 community system. Zero
logic, zero I/O, zero async — every other Rekindle crate depends on
this. Modules: `analytics`, `attachment`, `channel`, `cross_device_sync`,
`error`, `event`, `expression`, `governance`, `id`, `invite`,
`link_preview`, `permissions`, `presence`, `search`. These types
**replace** the v1.0 types that used to live in `rekindle-protocol`.

Dependencies: `serde`, `serde_json`, `thiserror`.

## Tier 2 — Cryptographic & Persistent Boundary

### rekindle-secrets

The **sole crate** that handles raw key material. Every secret type
implements `Zeroize + ZeroizeOnDrop`. No other crate in the workspace
should import `ed25519-dalek`, `x25519-dalek`, `aes-gcm`, or `hkdf`
directly. Modules: `derive` (HKDF-SHA256 derivations), `invite` (invite
signing keys), `keys` (Ed25519/X25519 wrappers with Zeroize), `mek`
(`MediaEncryptionKey`), `rotator` (deterministic blake3-based MEK
rotator selection), `sign` (sign/verify helpers), `sync_key`
(cross-device pairing key).

Dependencies: `rekindle-types`, `ed25519-dalek`, `x25519-dalek`,
`aes-gcm`, `hkdf`, `blake3`, `zeroize`, `libcrux-ml-kem` (PQC).

### rekindle-vault

Double-encrypted SQLCipher store that **replaces** the previous
`iota_stronghold` dependency. Two encryption layers:

1. **SQLCipher** (AES-256-CBC, page-level) encrypts the entire database
   file. Key is `BLAKE3-keyed("rekindle v1 vault-sqlcipher", master)`
   where `master = Argon2id(passphrase, salt)`.
2. **Per-entry AES-256-GCM** seals every `entries.ciphertext` row
   independently with a key derived from
   `BLAKE3-keyed("rekindle v1 vault-entry-gcm", master)`.

The 32-byte salt lives in `{vault_path}.salt` sidecar (plaintext — salts
only need to be unique per install). Modules: `key`, `schema`, `store`,
`error`. See [`decisions/0006-vault-replaces-stronghold.md`](../decisions/0006-vault-replaces-stronghold.md).

### rekindle-audit

BLAKE3 keyed hash chain for tamper-evident audit logging. Each
`AuditEntry` carries `prev_mac` + `mac` where
`mac = BLAKE3-keyed(key, prev_mac || cursor_le || payload_json)`.
Tampering with any byte of any entry's `payload_json` invalidates every
entry from that cursor forward. The MAC key lives in the vault under
`("audit", "mac_key")` and is generated on first unlock. Single module
`chain` plus re-exports. See
[`audit-chain.md`](audit-chain.md).

Dependencies: `blake3`, `serde`, `zeroize`.

## Tier 3 — Wire Format, Records, Local State

### rekindle-codec

Signed envelope construction, verification, dedup, and serialization for
the gossip mesh. Modules: `dedup` (sliding-window cache — envelope ID →
seen-at), `envelope` (`SignedEnvelope` build/verify, `CommunityEnvelope`
payloads).

Dependencies: `rekindle-types`, `rekindle-secrets`, `blake2`.

### rekindle-records

DHT record lifecycle management for the v2.0 universal SMPL schema
(`o_cnt: 0`, 255 member slots — the "Q-pid equation"). Houses the
durable write retry queue. Modules: `lifecycle` (open / close /
republish / refresh), `retry` (`WriteQueueHandle` — durable SMPL write
retry with backoff), `schema` (universal SMPL schema constants +
helpers).

Dependencies: `rekindle-types`, `rekindle-secrets`. **No `veilid-core`** —
this crate wraps Veilid abstractions but never calls `veilid_core` directly;
concrete I/O is plumbed in by the integration layer.

### rekindle-events

Reusable event primitives hoisted out of `rekindle-transport::subscriptions`
so `src-tauri`'s Phase 10 `event_resume` flow can use them without
pulling in `veilid-core`. Modules: `dedup` (`EventDedup`), `state`
(`SubscriptionState`), `state_effects` (effect application), `journal`
(`EventJournal` + `JournalCursor` for Tauri reconnect replay).

`rekindle-transport` keeps its `SubscriptionManager` (which **is** the
pipeline); this crate publishes the building blocks. See
[`event-dispatch.md`](event-dispatch.md).

### rekindle-idempotency

LRU+TTL cache that collapses duplicate mutating Tauri commands into one
side effect. Mutating commands like `send_dm` are easy to fire twice —
double-click, retry-on-blip, optimistic UI — and the cache stores each
command's response keyed by a frontend-generated UUID v7 so subsequent
calls short-circuit. Single module `cache` exporting `IdempotencyCache`
and `SharedIdempotencyCache`.

Dependencies: `moka`, `uuid`.

### rekindle-mek-rotation

Cascade MEK rotation protocol (architecture §10.5). When a member leaves
a community, the remaining peers rotate the Media Encryption Key so the
departed member can no longer decrypt new gossip. Rotation is
**decentralised**: every online peer independently computes the same
deterministic rotator via `blake3(departed_pseudonym || self_pseudonym)`,
and the lowest hash performs the rotation. Subsequent peers cascade down
the ordered list with grace windows if the elected peer is unreachable.

Modules: `election` (cascade-ranking + delay computation), `distribute`
(per-member MEK-wrap + gossip broadcast), `receive` (inbound MEK
acceptance), `rotate` (rotation entry points), `cache`
(`ChannelMekCache` trait), `deps` (`MekDistributeDeps` trait composing
I/O + persistence), `event` (`MekRotationEvent` to UI), `error`.

Parameterised over `MekDistributeDeps`, which **both** shells implement:
src-tauri's `services/mek_adapter.rs` against AppState / DbPool /
AppHandle, and the daemon's `daemon/mek_rotation/` against
`DaemonContext`. The daemon adapter delegates identity, Lamport counter,
membership and the online roster to its `GovernanceRuntimeDeps` adapter
rather than reading `DaemonContext` twice.

Two host-specific notes. The daemon queues rotations on a channel drained
by one worker instead of spawning per trigger: rotation sleeps through
the cascade levels, so it must not run inline, and detaching needs
`'static` state that a `&DaemonContext` handler cannot supply.
`voice_recipients` returns empty there — the daemon hosts no voice
engine, a recorded capability gap rather than a stub.

Delivery is `app_call` carrying a bare Cap'n Proto `CommunityEnvelope`
(`Caller::call_community_envelope`), deliberately unframed and unsigned
so both shells read the same bytes. That is safe for this payload alone:
`unwrap_mek` takes the sender's pseudonym public key as an ECDH input, so
the sender is authenticated by whether the ciphertext decrypts. The
transport admits no other unframed variant — see
`subscriptions/bare_envelope.rs`.

`MekPersist` is implemented on both tracks but **no orchestrator in this
crate calls `deps.persist()` yet**; the trait is wired at the edges and
unused in the middle.

### rekindle-analytics

Local-only community analytics (architecture §24.1). All metrics are
pure SQL aggregations against tables the device already owns; **nothing
leaves the device**. Every entry point takes a `rusqlite::Connection` +
`owner_key` + `community_id` — no AppState, no Tauri, no Veilid.
Modules: `activity_by_hour`, `buckets`, `channel_metrics`, `growth`,
`member_metrics`, `storage`. Constants `SEVEN_DAYS_MS`,
`THIRTY_DAYS_MS`, `ONE_DAY_MS` plus the daily bucket count for
timeseries align all metrics to the same 30-day x-axis.

Dependencies: `rekindle-types`, `rusqlite`.

## Tier 4 — Routing & Lifecycle

### rekindle-route

Private route lifecycle: allocation, refresh, peer route cache.
Modules: `cache` (`RouteCache` — per-peer route blob + TTL eviction),
`contexts` (per-purpose `RoutingContext` factories — priv route, safety
route, unsafe), `lifecycle` (`RouteLifecycle` — periodic refresh, dead-
route detection).

Dependencies: `rekindle-types`, `blake3`, `tokio`. **No `veilid-core`** —
contexts are passed in from above.

### rekindle-lifecycle

9-state application FSM hoisted from `rekindle-node::daemon` so the
Tauri shell can share the same machinery. Tracks where the application
is in its boot/login/shutdown cycle. Capability gates (`can_query`,
`can_write`, `can_unlock`) advertise which commands are safe; mutating
commands wrap their body in `TransportGuard::write` to reject calls in
states where the side effect can't be safely produced. Modules: `state`
(`AppLifecycle`, `LifecycleState`), `guard` (`TransportGuard`), `error`.
See [`lifecycle-fsm.md`](lifecycle-fsm.md).

Dependencies: `tokio`.

## Tier 5 — Gossip Mesh & Presence

### rekindle-gossip

Transport-agnostic gossip mesh primitives. Pure logic — does not call
`app_message` itself; the integration layer plumbs the broadcast
helpers into Veilid. Modules: `broadcast` (generic broadcast helpers),
`dedup` (`DedupCache` re-exported into `AppState`), `lamport` (clock
arithmetic plus the `MAX_LAMPORT_DRIFT` cap), `mesh` (`fanout_degree()`
— adaptive D selection: ≤20 → min(N, 6); 21–60 → 6; 61+ → 8),
`rate_limit` (token bucket, 10 msg/s floor), `mesh_broadcast`,
`peer_select`.

**Fan-out and TTL are one setting, not two.** The epidemic-broadcast
parameters are D as above *with* a 5-hop TTL (`rekindle-codec`'s
`envelope::DEFAULT_TTL`): the dedup cache plus the hop budget are what
let a sampled D still reach every member, so tuning either alone
changes coverage. Both tracks import these — `rekindle-gossip` for the
Tauri app, `rekindle-transport` for the daemon — rather than declaring
their own, because a mesh mixes peers from both.

Dependencies: `rekindle-types`, `rekindle-codec`, `rekindle-protocol`,
`rekindle-crypto`, `async-trait`.

Consumed by `rekindle-transport` (daemon track) as well as the Tauri
host — the downward edge from the daemon's Veilid boundary into these
Tier-5 primitives.

### rekindle-presence

Presence primitives plus the friend-presence + community-presence
orchestrators. Same chiral split as the other Phase 17–20 crates: pure
protocol logic parameterised over `Deps` traits, with the src-tauri
adapter providing AppState orchestration. Modules: `community`
(community presence + RSVP aggregation + gossip overlay rebuild),
`friend` (friend presence), `friend_sync`, `heartbeat`, `idle` (auto-
away decision logic), `poll_buckets` (steady/rapid tick scheduling),
`status` (status transitions), `deps`. See
[`presence.md`](presence.md).

Public surface includes `presence_poll_tick`, `compute_rebuild_plan`,
`compute_merged_roles`, `aggregate_event_rsvps`, plus timing constants
(`STEADY_TICK_INTERVAL_SECS`, `RAPID_TICK_INTERVAL_SECS`,
`STALE_HEARTBEAT_SECS`, `MAX_SYNC_ATTEMPTS`).

### rekindle-friendship

Three-tier inbox-scan coordinator. Friend requests land in a per-peer
DHT inbox; scanning every second wastes the network, scanning every
minute makes UX laggy. The coordinator fans three independent triggers
into one stream of scans:

1. **`watch_rx`** — `watch::Receiver<u64>` that fires on
   Veilid `ValueChanged`. ~1 s end-to-end when the watch is healthy.
2. **30-second poll** — backstop for silent watch death under route
   churn.
3. **Direct trigger** — `mpsc::Sender<()>` exposed via the Tauri command
   `friendship_scan_now`.

All three debounce on a 500 ms coalesce window. Modules: `coordinator`
(`InboxScanCoordinator`, `InboxScanner`, `ScanError`), `watch_trigger`,
`veilid_scanner`.

## Tier 6 — Governance

### rekindle-governance

**No I/O. No async. No side effects.** Takes `GovernanceEntry` variants
from all member subkeys, sorts by `(lamport, author_pseudonym)`, and
applies deterministic merge rules to produce a `GovernanceState`. Every
peer running the same merge on the same entries produces an identical
result — the CRDT convergence guarantee.

Modules: `merge` (the CRDT engine), `permissions` (reader-validates:
derive effective permissions for a member), `state` (`GovernanceState` —
channels, roles, members, bans, settings, …), `validate` (entry-level
validation — size, well-formedness). The `proptest-regressions/merge.txt`
file pins property-test seeds for the merge function — do not delete.

Dependencies: `rekindle-types` only.

### rekindle-governance-runtime

The async lifecycle layer on top of the pure CRDT. Hosts the operations
that touch the network and the local store: `origin` (community
creation), `bootstrap` (fetch origin + governance + members on join),
`join` + `join_stages` + `join_gate` (single-shot entry + per-stage
state machine), `segments` (Plate Gate segmentation for >255 members),
`apply` (writes that flow through `rekindle_governance::merge::apply`),
`dht_hydration` (rehydrate registry slots from the network),
`invite_secrets`, `membership_events`, `overflow`, `roles`, `event`,
`error`, `deps`.

Trait surface in `deps.rs` keeps Tauri / Veilid / SQLite / vault
behind the adapter at `src-tauri/src/services/governance_adapter.rs`.
The crate itself never imports `veilid-core`, `tauri`, `rusqlite`, or
`iota_stronghold`. See [`communities-governance.md`](communities-governance.md).

## Tier 7 — Self-Contained Features

### rekindle-channel

Community channel messaging — send, receive, threads, reactions,
expressions (custom emoji / soundboard / stickers), mentions,
notifications, polls, automod, slowmode. Layered on top of
`rekindle-mek-rotation` (per-channel MEK lookup), `rekindle-gossip`
(mesh broadcast), and `rekindle-protocol` (signed envelope encoding).
Modules: `send`, `receive`, `pipeline`, `stage`, `threads`, `reactions`,
`expressions`, `mentions`, `notifications`, `polls`, `automod`, `event`,
`deps`, `error`. See [`communities-channels.md`](communities-channels.md).

### rekindle-dm

Direct messages and group DMs (architecture §27). DMs are SMPL records
with `o_cnt: 0`, exactly 2 member subkeys, and a MEK derived
deterministically via X25519 ECDH between the two identity keys (no
separate key-exchange round trip). Group DMs wrap the MEK per recipient.
Pure logic — no DHT, no Tauri. The `src-tauri/services/dm/` layer wires
it to Veilid and SQLite via the `DmStore` trait (`SqliteDmStore` impl
included). Modules: `invite` (`DmInvite`, `GroupDmInvite`,
`GroupDmParticipant`), `mek` (`derive_dm_mek` → HKDF, `ratchet_dm_mek`,
`DmMekChain`), `error`.

### rekindle-calls

Direct calls (architecture §10.10, "Chiralgrams") — peer-to-peer voice
and video between two friends. Derives a 32-byte `call_key` via X25519
ECDH plus HKDF-SHA256 (`HKDF_INFO = b"rekindle-call-key-v1"`,
salt = call ID), giving both sides the same shared secret. The `state`
module tracks ringing/answered/missed state and the `signaling::CallRegistry`
trait powers the Tauri-side `AppState.active_calls` (Phase 14.q).
`rekindle-voice` consumes `call_key` to encrypt frames over
`app_message`; this crate has no Veilid, no audio I/O, no Tauri.

Dependencies: `rekindle-types`, `rekindle-utils`, `x25519-dalek`, `hkdf`,
`sha2`, `aes-gcm`, `tokio`, `async-trait`.

### rekindle-files

Lost Cargo: chunked Merkle-verified P2P file delivery (architecture
§28.9). Per-file FEK pattern (Signal/Matrix style), `AttachmentBitmap`
for swarm fetch, filesystem cache with synchronous LRU eviction,
BLAKE3 chunk hashes. Modules: `cache` (`ChunkCache`, `PinnedSet`, LRU
eviction), `chunker`, `download`, `upload`, `expression_fetch`,
`manifest` (`AttachmentManifest` — chunk hashes, size, MIME), `pinned`,
`serve`, `verify` (Merkle verification), `deps`, `error`, `fek`.

### rekindle-link-preview

Sandboxed OpenGraph fetcher (architecture §28.8). Single public async
function `fetch_link_preview`. Hard limits: 5 s timeout, 256 KB body
cap, plain text/HTML only, max 5 redirects, custom `User-Agent`.

### rekindle-video

Video and screen-share fragmentation / reassembly (architecture §10.6).
Pure logic — no codec FFI, no Tauri, no I/O. The actual VP9 encode /
decode plugs in via the `VideoCodec` trait at the application layer;
this crate handles only the on-the-wire framing (≤28 KB payload chunks,
FEC-friendly indexing, per-stream reassembly buffer with bounded
memory). Modules: `fragment` (`fragment_frame`,
`fragment_frame_with_fec`, `reconstruct_frame`), `reassembler` (per-
stream buffer).

Dependencies include `reed-solomon-erasure` for FEC.

## Cross-Cutting Integration Crates

### rekindle-protocol

Veilid networking, DHT record management, Cap'n Proto serialization, and
routing for the **desktop app**. Hosts the v1.0 `MessageEnvelope` /
`MessagePayload` types still used for 1:1 friend traffic (DM invites,
friend requests, relay payloads, presence inline updates).

Top-level modules: `node` (`RekindleNode` lifecycle), `routing` (private
route allocation, peer route import), `peer` (address resolution),
`capnp_codec` (encode / decode helpers), `messaging` (`envelope`,
`sender`, `receiver`), `dht` (`profile`, `presence`, `friends`,
`conversation`, `account`, `mailbox`, `channel`, `short_array`, `log`,
`community/{envelope, manifest, member_registry, channel_record,
audit_log, automod, onboarding, permissions_v2, types}`), `error`.

`MessagePayload` variants cover: `DirectMessage`, `ChannelMessage`,
`FriendRequest/Accept/Reject`, `ProfileKeyRotated`, `PresenceUpdate`,
`Unfriended`, `RelayOffer/Withdraw/Ack/Envelope`,
`DmInvite/Accept/Decline`, `GroupDmInvite`, `DmLeave`,
`RegisterPushRelay`, `UnregisterPushRelay`, `WakeNotify`,
`StatusRequest/Response`.

### rekindle-crypto

Cryptographic operations including Ed25519 identity, Signal Protocol
session handling, group MEK primitives, and HKDF-derived DHT record
keys. Modules: `identity` (Ed25519 keypair, sign / verify, hex
helpers), `keychain` (`Keychain` trait, vault / key constants),
`dht_crypto` (`DhtRecordKey` — account/conversation key derivation +
XChaCha20-Poly1305 encrypt / decrypt), `group/{media_key, pseudonym}`
(`MediaEncryptionKey` with generation tracking; community pseudonym
derivation), `signal/{session, prekeys, store, memory_stores,
test_stores}` (X3DH + Double Ratchet session manager — vault-backed
Signal stores now, previously Stronghold-backed).

### rekindle-game-detect

Cross-platform game detection: process scanning + JSON game database +
launcher integration. Modules: `scanner`, `database`, `launcher`,
`rich_presence` (server info + elapsed time — currently type stubs
pending wire-up), `platform/{linux, macos, windows}`.

### rekindle-voice

Voice chat pipeline. `cpal::Stream` is `!Send` on macOS, so capture and
playback live on dedicated OS threads and bridge to Tokio via `mpsc`
channels. Modules: `capture`, `playback`, `codec` (`OpusCodec` —
48 kHz mono, VoIP mode, 32 kbps, in-band FEC), `audio_processing`
(RNNoise denoising + AEC3 echo cancellation + VAD), `audio_thread`,
`device`, `jitter` (`JitterBuffer` — adaptive, BTreeMap by sequence),
`mixer`, `transport`, plus group / MCU / mutual-aid SFU machinery for
calls of more than four participants.

Voice packets use the same 3-hop Tor-class `SafetySelection::Safe`
route as every other path (`Stability::LowLatency` within the
anonymity floor), so the sender's node identity is never exposed;
mouth-to-ear latency is budgeted to the ITU-T G.114 interactive band.

### rekindle-sync

Cross-device sync: fetch, gap detection, history, warming, and DHT
watching. Modules: `fetch` (fetch missing subkeys), `gap` (gap
detection), `history` (catch-up), `inspect` (network sequence inspection
— cheaper than full fetch), `verify`, `warming` (record warming on
first interest), `watch` (`watch_dht_values` orchestration).
Parameterised over a `SyncDeps` trait.

### rekindle-utils

Time helpers (`now_ms`, `now_secs`, monotonic timestamps). Single
module `time`. Zero external deps beyond `std`.

### rekindle-e2e-server

HTTP IPC bridge that exposes Tauri commands over `localhost:3001` so
Playwright tests can drive the real Rust backend without the Tauri
webview. Used when `VITE_E2E=true`. Single binary `e2e_server`.

## Daemon and CLI Track

### rekindle-transport

The **sole Veilid boundary** on the daemon track. Every other crate in
the track depends on `rekindle-transport` and never imports
`veilid-core` directly. The crate is split into two `veilid_core`-aware
modules and a body of pure logic:

- `broadcast/` — outbound: sends, DHT writes, route management, node
  lifecycle. The only outbound module that imports `veilid_core`.
- `subscriptions/` — inbound: event dispatch, DHT watches, value-change
  routing. The only inbound module that imports `veilid_core`.
- Everything else (`operations/`, `payload/`, `crypto/`, `session/`,
  `community/`, `gossip.rs`, `frame.rs`, `query.rs`, `handler.rs`, …)
  contains zero `veilid_core` imports.

Public API re-exports include `TransportNode`, `Sender`, `RouteManager`,
`PeerRegistry`, `DhtStore`, `InboundHandler`, `GossipMesh`,
`SignalSessionManager`, `Session`, `QueryEngine`, plus per-feature
operation modules (`operations::{community, channel, dm, friend, voice,
mek, presence, roles, moderation, invites, identity}`).

Dependencies: `rekindle-types`, `rekindle-utils`, `rekindle-events`,
`rekindle-secrets`, `rekindle-crypto`, `rekindle-calls`, `rekindle-route`,
`veilid-core`, `ed25519-dalek`, `x25519-dalek`, `aes-gcm`, `hkdf`,
`blake3`, `bitflags`, `postcard`.

### rekindle-node

The Rekindle **daemon**. Owns the `TransportNode`, manages persistent
state, and serves CLI/TUI/Tauri frontends plus automation bots over a
Noise-IK encrypted IPC bus. Modules: `validation` (request validation),
`ipc/` (`server`, `client`, `transport`, `framing`, `noise` (Noise IK
handshake), `noise_keys` (OS keyring for daemon long-term key),
`protocol` (`IpcRequest` / `IpcResponse`), `registry` (UCred-pinned
client registry), `message`), `daemon/` (`handler`, `community_rpc`,
`governance_rpc`, `friend_inbox`, `event_router`, `dispatch/`),
`state/` (session, config, path).

`rekindle-node` depends on `rekindle-transport`, `rekindle-types`,
`rekindle-lifecycle`, `snow` (Noise), `keyring`, `rustix` (for safe
`SO_PEERCRED`), and `sd-notify` (systemd `READY=1` + watchdog). It
never imports `veilid-core` directly.

### rekindle-cli

CLI and TUI for the daemon track. Binary name `rekindle-cli` (renamed
from `rekindle` to avoid collision with the desktop app's
`src-tauri/` binary). Every CLI command sends an `IpcRequest` over the
Noise-IK bus and renders the `IpcResponse` — the CLI never touches
`TransportNode`, `Session`, or the OS keyring directly. Modules: `cli/`
(clap subcommands), `tui/` (ratatui interactive mode), `views/` (12
screen renderers), `output/` (JSON / table), `config/`, `transport`
(IPC client wrapper), `node_daemon` (embedded daemon mode behind the
`daemon` feature), `identity`, `keys`, `network`, `presence`,
`friends`, `dm`, `community`, `channel`, `governance`, `voice`,
`helpers`, `error`.

Default features: `tui` (ratatui + crossterm + textarea + arboard) and
`daemon` (embeds the daemon in the same binary for solo-developer
setups, gated on `rekindle-transport`, `snow`, `sd-notify`, `rustix`,
`postcard`). Both can be disabled for a minimal CLI-only build.
