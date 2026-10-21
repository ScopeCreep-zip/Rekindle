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
`error`, `event`, `expression`, `governance`, `id`, `image_kind`, `invite`,
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
`error`, and `typed::{community_keys, mek, audit, owner_keys}`: one module of
helpers per kind of secret, so every host stores them the same way (plan
C5.4). See [`decisions/0006-vault-replaces-stronghold.md`](../decisions/0006-vault-replaces-stronghold.md).

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

Rekindle's wire types and their encodings, with no Veilid (plan C8,
ADR 0014). Every tier crate that needs a wire type depends on this crate, not
on `rekindle-protocol`. Modules:
- the generated `*_capnp` modules (built from `schemas/` by this crate's
  `build.rs`), `capnp_codec` and `capnp_envelope`;
- `community` (`CommunityEnvelope`, `ControlPayload`, the community
  `SignedEnvelope`, governance and metadata types, `channel_record` page
  types and their signed encoding);
- `capnp_codec::direct` (the 1:1 wire, plan E2.3: `DmFrame`,
  `DmContent`, `SealedContent`, the `DmTypeId` registry and one body type
  per kind; schema `schemas/direct.capnp`);
- `ingress` (the `'D'` 1:1 and `'G'` gossip datagram tags and `classify`;
  an untagged datagram is dropped);
- `message` (invite blobs and the community RPC DTOs; `MessageEnvelope` and
  `MessagePayload` are deleted);
- `friend` (`FriendEntry`);
- `envelope` (the gossip `SignedEnvelope` build/verify);
- `dedup` (sliding-window cache: envelope ID → seen-at);
- `error` (`CodecError`, which converts into `ProtocolError`).

Dependencies: `rekindle-types`, `rekindle-secrets` (including its
`ed25519_dalek` re-export), `rekindle-utils`, `capnp`, `serde_json`, `blake2`,
`blake3`, `base64`.

### rekindle-db

The storage foundation every host shares (plan C5): the SQLite schema
(`schema/001_init.sql`, `SCHEMA_VERSION`), `open` (pragmas, version check,
reset), the `Db` handle every query goes through, the repositories for
the tables more than one host uses (`repo::{identity, friends,
pending_requests, communities, members, governance_cache, audit}`),
`SqliteFriendStore`, and `lock::NodeLock` (one node per data root).
`paths` re-exports `rekindle_utils::paths::DataRoot`. Repository
functions take `&rusqlite::Connection` so several share one transaction;
`cargo xtask check-sqlite` keeps SQL here, in `rekindle-vault` and in
`rekindle-asql`.

The 1:1 ratchet's storage (plan E2.3): `ratchet::{initiate_and_record_outbound,
encrypt_and_record_outbound, decrypt_and_record_inbound, accept_init}` run
`rekindle_crypto::signal::SessionRecord` inside `Db::call`, one transaction per
message before any send, so the single `Db` thread serializes every message for a
peer, `accept_init` consuming a one-time key and refilling its slot in the
same transaction (B8) and `discard_pending_init` for a `PreKeyRejected`
(B6); `prekeys` over `signal_prekeys` and the `prekey_slots_used`,
`prekey_rejections` and `prekey_lr_inits` tables (plan E2.4: store a minted
set, apply maintenance, the unlock reconciliation's `published_values`,
`status`, the rejection limits and the archive sweep);
`repo::signal_identities` (verification and `VerifiedKeyChanged`) and
`repo::unprocessed` (the D24 receive pipeline's decrypted, unapplied messages).

Dependencies: `rekindle-types`, `rekindle-audit`, `rekindle-crypto` (the ratchet
and prekey material; cross-cutting, plan E2.3), `rekindle-utils`,
`rekindle-asql`, `rusqlite` (bundled SQLCipher, as `rekindle-vault`
builds it), `hex`, `serde`, `serde_json`, `thiserror`, `tracing`, `zeroize`.

### rekindle-records

DHT record lifecycle management for the v2.0 universal SMPL schema
(`o_cnt: 0`, 255 member slots — the "Q-pid equation"), and the Veilid-free
ports. Modules: `chain` (plan E2.5, primitive N2: the append-only chain on
`&dyn RecordPort` — a participant record's `HEAD` names the newest sealed
DFLT(1) page, written once with its owner keypair dropped, each page names the
one before it, and `read_after(cursor)` walks back to the cursor and stops at
`RETIRE_THROUGH`, reporting a retired gap; pages are Cap'n Proto
`schemas/chain.capnp` through `rekindle-codec`), `lease` (lease ids, subkey sets, community leases),
`lifecycle` (open / close / republish / refresh), `port` (plan C9.1: the
`RecordPort`/`RecordStore`, `RoutePort` and `AppMessagePort` traits, with
keys, keypairs and route ids as Veilid's text, `PortEvent`, `PortSet` and
`PortError`; `rekindle-protocol` implements them over Veilid), `retry`
(`WriteQueueHandle` — durable SMPL write retry with backoff), `schema`
(the named record schemas as `RecordSchema` builders, and the universal
SMPL constants).

Dependencies: `rekindle-types`, `rekindle-codec`, `async-trait`, `serde`,
`serde_json`, `thiserror`, `tokio` (`sync`). **No `veilid-core`**, and no `veilid_core`
named anywhere in it (`cargo xtask check-veilid-dht-calls`).

### rekindle-events

Reusable event primitives hoisted out of `rekindle-transport::subscriptions`
so `src-tauri`'s Phase 10 `event_resume` flow can use them without
pulling in `veilid-core`. Modules: `dedup` (`EventDedup`), `state`
(`SubscriptionState`), `state_effects` (effect application), `journal`
(`EventJournal` + `JournalCursor` for Tauri reconnect replay), `typing`
(plan E2.3: `TypingExpiry`, the receiver's lapse of an unrefreshed start,
and `TypingSessions`, the sender's refresh and idle stop, keyed by
`TypingContext` over the Host's clock). Typing lives here because it is
event state both tracks need without a transport: the Host drives the 1:1
deadlines (`rekindle_transport::operations::{typing, dm::typing}`), and
E2.6's group typing (`rekindle-dm`, Tier 7) and E3.4's channel typing
(with its own per-start TTL) use the same two types.

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
src-tauri's `services/mek_adapter.rs` against AppState / Db /
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

### rekindle-image

The one image decoder (plan E7.11). Every image Rekindle takes from
outside (our avatar now; community avatars and banners when E3.1 ports
them; skin pictures in E7.7.1) goes through
`decode_limited(bytes, declared: ImageKind, &DecodeLimits) -> Result<Decoded,
ImageError>`. The caller declares the kind (`rekindle_types::image_kind::
ImageKind`, PNG, JPEG or GIF: the file picker's MIME type, a file's
extension, a skin's manifest); the magic bytes must match it
(`ImageError::KindMismatch`) and only that kind's decoder runs, so a
crafted file never picks its own decoder (the FORCEDENTRY path). The header
is read and its sides and pixel count checked before any pixel buffer is
allocated (`ImageError::TooLarge`); the decode then runs under
`image::Limits` (`max_image_width`, `max_image_height`, `max_alloc`, which
`image` passes to the `png` crate as `png::Limits`). `Frames::First` reads a
GIF's first frame; `Frames::AtMost(n)` reads every frame, counting them and
summing their buffers against `max_alloc`. Output is re-encoded with no
metadata carried over: `encode_png`, `encode_gif`, `encode_webp` (lossless),
and `fit_within` scales a picture down. WebP is never decoded.

Dependencies: `rekindle-types`, `image` (`png`, `jpeg`, `gif`, `webp`
features: pure-Rust `png`, `zune-jpeg`, `gif`, `image-webp`), `thiserror`.
`#![forbid(unsafe_code)]`; no Veilid, no Tauri.

## Tier 4 — Routing & Lifecycle

### rekindle-route

Route policy, the peer route cache and the owner of our own routes.
Modules: `cache` (`RouteCache` — per-peer route blob + TTL eviction),
`contexts` (per-purpose routing-context factories), `lifecycle` (the
watchdog cadence and the peer-cache TTL), `own_routes` (`OwnRoutes`: our
routes are allocated, healed and released here, plan C7.9 — one owner per
node, a general and a media class, no fallback between them, single-flight
and ready-gated allocation with backoff; generic over a `RouteAllocator`,
which `rekindle_protocol::own_routes::VeilidRouteAllocator` implements over
Veilid and a simulator can fake, plan C9.1), `profile` and `relay`.

Dependencies: `rekindle-types`, `rekindle-records` (`RouteClass`,
`RouteState`), `rekindle-lifecycle` (`SessionScope`), `async-trait`,
`blake3`, `parking_lot`, `tokio`, `tokio-util`, `tracing`. **No
`veilid-core`** — the allocator is passed in from above.

### rekindle-lifecycle

9-state application FSM hoisted from `rekindle-node::daemon` so the
Tauri shell can share the same machinery. Tracks where the application
is in its boot/login/shutdown cycle. Capability gates (`can_query`,
`can_write`, `can_unlock`) advertise which commands are safe; mutating
commands wrap their body in `TransportGuard::write` to reject calls in
states where the side effect can't be safely produced. Modules: `state`
(`AppLifecycle`, `LifecycleState`), `guard` (`TransportGuard`), `error`,
and `scope` (`SessionScope`: the tasks of one session — a desktop login,
a daemon unlock, a community, a call — spawned, named, cancelled and
waited for together; plan C4).
See [`lifecycle-fsm.md`](lifecycle-fsm.md).

Dependencies: `tokio`, `tokio-util` (`rt`), `futures-util` (`catch_unwind`),
`parking_lot`.

### rekindle-record-pool

The session's DHT record pool (plan C7.2, D4; ADR 0021): `RecordPool`, the
only code that opens, closes, watches, reads or writes a DHT record. It runs
on an `Arc<dyn rekindle_records::port::DhtBackend>` and is the
`RecordPort` the transport node uses. Modules: the crate root (leases over
`rekindle_records::lease::LeaseTable`, the one retry layer within
`RETRY_BUDGET`, admit/drain/teardown/end), `call` (every backend call a task
on the pool's `SessionScope`; a caller that leaves drops only its wait),
`io` (get, inspect, honest `set`), `read` (which subkeys need a network
read), `durable` (held writes and their re-push, `Settled`), `watch` (watch
plans and the one watch-death owner: re-arm with backoff and forgiveness on
`tokio::time`), `closer` (`RecordCloser`: an ended session's records closed
after its calls finish), `keepalive` (record warming cadence) and `port`
(the `RecordPort` impl). The Veilid backend is
`rekindle_protocol::dht::backend::VeilidDht`; `rekindle_protocol::dht::pool`
re-exports this crate. Its unit tests run on an in-crate test backend on
tokio's paused clock.

Dependencies: `rekindle-records`, `rekindle-lifecycle` (`SessionScope`),
`rekindle-utils` (`RandomSource`), `async-trait`, `futures`, `parking_lot`,
`tokio`, `tokio-util`, `tracing`. **No `veilid-core`** — listed in
`VEILID_FREE` (`cargo xtask check-veilid-dht-calls`).

### rekindle-outbox

The one owner of every send intent (plan E2.5, D22). A send is a row in an
ordered stream `<kind>:<scope>` — kinds `dm`, `friend_handshake`,
`dht_write`, `channel_post` and `media_key` (`group_dm` arrives at E2.6,
`signaling` at E4.1) — written with its local echo in one transaction and
attempted by `next_at` until it settles. Modules: `state` (one vocabulary,
`queued < sent < delivered < read` plus `failed`, merged monotonically;
`delivered`/`read` only from a peer ack, `failed` only over an unsettled
state), `stream` (`StreamKind`, each kind's `OutboxRetry`: exponential
backoff with equal jitter drawn from the injected `RandomSource`), `deps`
(`OutboxDeps`: storage, attempt, clock, RNG and the state-change publish)
and `engine` (`Outbox`: `enqueue`, `pump`, `run`, `acknowledge`, `retry`
under the same client id, `discard`, `receive` dedupe, `unsettled`). Each
row's deadline is `DELIVERY_HORIZON` (D24) unless its stream sets an
earlier end; past it the row fails and wedges its stream head until Retry
or Discard. Storage is `rekindle_db::repo::outbox`; `OutboxDeps` is
implemented once in `rekindle-node/src/host/adapters/outbox/`. No SQL, no
Veilid.

Dependencies: `rekindle-types` (`SendState`), `rekindle-utils` (`Clock`,
`RandomSource`), `rekindle-crypto` (`horizon::DELIVERY_HORIZON`),
`async-trait`, `thiserror`, `tokio`, `tokio-util`, `tracing`.

## Tier 5 — Gossip Mesh & Presence

### rekindle-gossip

Transport-agnostic gossip mesh primitives. Pure logic — does not call
`app_message` itself; the integration layer plumbs the broadcast
helpers into Veilid. Modules: `broadcast` (generic broadcast helpers),
`dedup` (`DedupCache` re-exported into `AppState`), `lamport`
(re-export of `rekindle_types::lamport::LamportClock`: checked
increment, receive rule clamped by `MAX_LAMPORT_DRIFT`), `mesh` (`fanout_degree()`
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
`compute_merged_roles`, `PRESENCE_ROW_CAP`, plus timing constants
(`STEADY_TICK_INTERVAL_SECS`, `RAPID_TICK_INTERVAL_SECS`,
`STALE_HEARTBEAT_SECS`, `MAX_SYNC_ATTEMPTS`).

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

Group DMs (architecture §27.2): SMPL records with `o_cnt: 0`, one member
subkey each, and a random MEK wrapped per recipient. The 1:1 SMPL DM (2
member subkeys, an ECDH MEK) was deleted by plan E2.3; E2.6 replaces the
group path with sender-owned keys.
Pure logic — no DHT, no Tauri. The `src-tauri/services/dm/` layer wires
it to Veilid and SQLite via the `DmStore` trait (`SqliteDmStore` impl
included). Modules: `invite` (`GroupDmInvite`, `GroupDmParticipant`),
`mek` (`ratchet_dm_mek`, `DmMekChain`), `error`.

### rekindle-calls

Direct calls (architecture §10.10, "Chiralgrams") — peer-to-peer voice
and video between two friends. Derives a 32-byte `call_key` via X25519
ECDH plus HKDF-SHA256 (`HKDF_INFO = b"rekindle-call-key-v1"`,
salt = call ID), giving both sides the same shared secret. The `state`
module tracks ringing/answered/missed state and the `signaling::CallRegistry`
trait powers the Tauri-side `AppState.active_calls` (Phase 14.q).
The derived secret is the call's SFrame scope secret
(`CallState.media_secret`), held with our sender state
(`CallState.media_sender`); `rekindle-voice` reads both through
`MediaKeySource` to seal frames. This crate has no Veilid, no audio I/O,
no Tauri.

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
routing for the **desktop app**. The v1.0 `MessageEnvelope` /
`MessagePayload` types and the `messaging` module (`envelope`, `sender`,
`receiver`, `signing`, `replay`) are deleted: the 1:1 wire is `DmFrame` in
`rekindle-codec` (plan E2.3), sent and opened by `rekindle-transport`
(`operations::{dm, direct}`).

Top-level modules: `node` (`RekindleNode` lifecycle), `own_routes`
(private route allocation), `ports`, `veilid_config`, `dht` (`profile`, `prekeys`, `friends`,
`account`, `mailbox`, `pool`, `route_imports`,
`community/{member_registry, channel_record}`), `error`.

### rekindle-crypto

Cryptographic operations including Ed25519 identity, the Signal Protocol
state machines (pure; persistence is `rekindle-db`'s), group MEK
primitives, and HKDF-derived DHT record
keys. Modules: `identity` (Ed25519 keypair, sign / verify, hex
helpers), `dht_crypto` (`DhtRecordKey` — account key derivation +
XChaCha20-Poly1305 encrypt / decrypt), `group/{media_key, pseudonym}`
(`MediaEncryptionKey` with generation tracking; community pseudonym
derivation), `signal/{dr, record, pqxdh, prekeys,
prekey_material, prekey_record}` (the spec Double Ratchet, the pure `SessionRecord`,
PQXDH rev. 3 with SKec/SKscka, the in-memory `PreKeyBundle` assembled from
a verified record, the prekey minting `rekindle-db`'s prekeys run, and the
prekey record's ids, signing and pure decisions `plan_maintenance` (B1) and
`pick_slot` (B2); plans E2.2-E2.4. `SignalSessionManager`, the
non-spec ratchet and the vault-backed Signal stores are deleted), `fingerprint` (`SafetyNumber`: Signal's 5200×SHA-512 numeric
fingerprint and its scannable form, E2.3) and `horizon` (`DELIVERY_HORIZON`,
the one constant every delivery retention bound derives from, D24).

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

Shared utilities, most behind features so a consumer pulls only what it
uses: `time`, `hash`, `random`, `text` (always on); `retry`
(`retry` feature); `log_scrub` (`log-scrub`); `config_layers` (`config`);
and `paths` (`paths`): `DataRoot`, the directories every host and frontend
resolves its files in, on the `dirs` release Tauri uses. It lives here,
not in `rekindle-db`, because the thin frontends must not link the
backend (`check-frontend-boundaries`).

### rekindle-e2e-server

HTTP IPC bridge that exposes Tauri commands over `localhost:3001` so
Playwright tests can drive the real Rust backend without the Tauri
webview. Used when `VITE_E2E=true`. Single binary `e2e_server`.

### rekindle-netsim

A seeded, deterministic network simulator (plan C9.3), `publish = false`
and Veilid-free. It implements the ports of `rekindle_records::port` in
memory: `FakeRecords`/`FakePool` (Veilid's record rules: publish on first
write, schema-admitted writers checked through `parse_keypair_text`,
sticky writers via `rekindle_records::lease::LeaseTable`, local seq + 1 and
supersede, per-node local copies and watches), `FakeRoutes` over the real
`rekindle_route::own_routes::OwnRoutes` with a `FakeAllocator`, and
`FakeMessages` (size cap, call timeout). `SimClock` and `SimRandom` are
injected into each Host (plan C9.2). A pump delivers the events in flight in
an order drawn from the seed and records a semantic `Trace`. Faults:
try again, offline, stale reads, partition and heal, dropped messages,
unanswered calls, clock skew, watch and route death, a modified client's
write, a same-seq conflict. `rekindle-node` takes it as a dev-dependency
only; its scenarios are in `crates/rekindle-node/tests/netsim/`.

Dependencies: `rekindle-records`, `rekindle-route`, `rekindle-crypto`,
`rekindle-secrets`, `rekindle-types`, `rekindle-utils`, `async-trait`,
`base64`, `blake3`, `parking_lot`, `rand`, `tokio`, `tracing`.

## Daemon and CLI Track

### rekindle-transport

One of the four crates allowed to link veilid-core (ADR 0014:
`rekindle-protocol`, `rekindle-transport`, `rekindle-node`, and
`rekindle-desktop` until F1), checked transitively by
`cargo xtask check-boundaries`. It links Veilid only through
`rekindle-protocol` and names no Veilid type (plan C9.1): `TransportNode`
is built from a `PortSet` (`rekindle_records::port`) — started by
`TransportNode::start` over `rekindle_protocol::ports::VeilidPorts`, handed
the desktop's ports by `adopt_outbound`, or a simulator's by `from_ports`.

- `broadcast/` — outbound: sends (`AppMessagePort`), DHT writes
  (`RecordPort`), route management (`RoutePort`), node lifecycle.
- `subscriptions/` — inbound: dispatch of `PortEvent`s, DHT watches,
  value-change routing.
- Everything else (`operations/`, `payload/`, `crypto/`, `session/`,
  `community/`, `gossip.rs`, `frame.rs`, `query.rs`, `handler.rs`, …) is
  business logic over those two.

Public API re-exports include `TransportNode` (its routes via
`TransportNode::routes`), `Sender`,
`PeerRegistry`, `DhtStore`, `InboundHandler`, `GossipMesh`,
`Session`, `QueryEngine`, plus per-feature
operation modules (`operations::{community, channel, dm, direct, friend,
prekeys, voice, mek, presence, relay, identity, …}`; `prekeys` keeps our
prekey record (publish, the unlock reconciliation, B1 maintenance,
`PrekeyStatus`), picks from a friend's (B2/B3), and sends and answers
`PreKeyRejected` (B5–B7); `direct` opens inbound `DmFrame`s:
`frame`, `handlers`, the sealed-frame `replay` guard).

Dependencies: `rekindle-types`, `rekindle-utils`, `rekindle-events`,
`rekindle-secrets`, `rekindle-crypto`, `rekindle-route`,
`rekindle-protocol`, `rekindle-records`, `ed25519-dalek`, `aes-gcm`, `hkdf`,
`blake3`, `bitflags`, `postcard`.

### rekindle-node

The Rekindle **daemon**. Owns the `TransportNode`, manages persistent
state, and serves CLI/TUI/Tauri frontends plus automation bots over a
Noise-IK encrypted IPC bus (`rekindle-ipc`). It ships as the `rekindled`
binary (`src/bin/rekindled.rs`). Modules: `host/` (composition root:
`lock` — one daemon per data root, `policy` — the fail-closed admin-policy
loader, `bus_key`, `watchdog`, `identity` — the identity store both hosts
use, `session` — the daemon's session projected from it, `adapters` — the
friend and 1:1 operations' Host and the MEK store over the vault), `validation` (request validation),
`daemon/` (`handler`, `community_rpc`, `governance_adapter`, `gossip_adapter`,
`presence_adapter`, `mek_rotation`, `dispatch/`), `state/` (the audit log, the unlocked signing key).

`rekindle-node` depends on `rekindle-ipc`, `rekindle-transport`,
`rekindle-types`, `rekindle-lifecycle`, `snow` (the bus keypair),
`rekindle-vault` and `rekindle-db` (the identity store, `host::identity`),
`rustix`, and `sd-notify` (systemd `READY=1` + watchdog). It
never imports `veilid-core` directly.

### rekindle-ipc

Tier 3, `#![forbid(unsafe_code)]`. The IPC bus every frontend and the
daemon link: wire protocol (`IpcRequest` / `IpcResponse`), Noise IK
handshake and transport, framing, `BusClient`, `BusServer`, the
`ClearanceRegistry`, the `EventRouter`, the drop-oldest media queue, and
the platform transport — Unix domain sockets with a same-UID peer check,
or (Windows) a named pipe whose protected DACL admits only its owner and
LocalSystem, built through `interprocess`'s safe API. Depends on
`rekindle-types`, `rekindle-utils`, `snow`, tokio; no Veilid, no storage.

### rekindle-client

What every frontend shares: `DaemonClient` over `rekindle-ipc`,
`spawn::connect_or_spawn` (starts the sibling `rekindled` on demand),
`config::load` (the layered `config.toml`), display formatters, the
scrubbed file logger and `ClientError`. No Veilid, no storage.

### rekindle-cli

The command-line frontend, binary `rekindle`. Every command sends an
`IpcRequest` over the Noise-IK bus and renders the `IpcResponse`.
Modules: `cli/` (clap), `output/` (text / JSON / JSONL), `watch`
(streaming commands), `config/` (`rekindle config`), the per-domain
command files, `helpers` (prompts), `error` (exit codes). No features.

### rekindle-tui

The terminal UI, binary `rekindle-tui`: ratatui dashboard, channel,
DM, voice, friend, doctor and community views, rendered from the
daemon's event stream; `keymap/` holds the default keybindings.

`cargo xtask check-frontend-boundaries` verifies that none of these three
links `veilid-core`, `rekindle-transport`, `rekindle-protocol`,
`rekindle-vault`, `rekindle-db`, `rekindle-node` or `rekindle-desktop`.
