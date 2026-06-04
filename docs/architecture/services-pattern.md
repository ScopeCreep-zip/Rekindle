# Services Pattern

`src-tauri/src/services/` is the largest directory in the Tauri shell
— around 229 `.rs` files. It is also the most patterned. Every file
in there belongs to one of three categories, and the categories are
the boundary between "Tauri-runtime glue" and "pure protocol logic
that could run anywhere."

This doc names the pattern, explains why it exists, and lists where
each category lives so that newcomers can find the right insertion
point without grep-walking 229 files.

## The three categories

### 1. Runtime (`*_runtime.rs`)

A runtime file orchestrates a single Tauri-side flow that touches
multiple stores: the DHT, governance state, AppState, SQLite, and
events. Its body is the legitimate "Tauri-runtime glue" — code that
cannot move to a crate because it depends on `AppState`, the
`AppHandle`, and the Tauri command boundary.

Every runtime file follows the same shape:

1. Permission check via `commands::community::helpers::require_permission`.
2. Read AppState through `state_helpers::*`.
3. Compute the next state (often by delegating to a pure-logic
   function in the same `services/community/*.rs` file or in a Tier-7
   crate like `rekindle-channel`).
4. Persist via `db_helpers::db_call`.
5. Emit through `event_dispatch::emit_journaled`.

Examples:

- `services/auth_runtime.rs` — login flow task spawning.
- `services/community_channel_runtime.rs` — channel creation: DHT
  record creation + governance entry write + AppState mutation +
  SQLite persist + event emit.
- `services/community_lifecycle_runtime.rs` — community
  bootstrap / rejoin orchestration.
- `services/community_mek_local_rotate.rs` — local MEK rotation
  bookkeeping triggered by `rekindle-mek-rotation` deciding this
  peer is elected.

The runtime never makes protocol decisions in its body. A runtime
that finds itself computing "who is the next MEK rotator" or "what
permission bits does this role have" is a smell — that logic belongs
in `rekindle-mek-rotation` or `rekindle-governance`, with the runtime
calling into it.

### 2. Adapter (`*_adapter/`)

An adapter directory implements a `Deps` trait defined in one of the
Tier-3-through-7 crates against the live `AppState` + `AppHandle` +
`DbPool`. It is the **Schwarzschild boundary**: the only place in
`src-tauri/` that both holds an `Arc<AppState>` and constructs
`veilid_core::*` types on the crate's behalf.

Every adapter follows the same internal split (see
`services/channel_adapter/` as the reference shape):

| File | Contents |
|---|---|
| `mod.rs` | The adapter struct (`pub struct ChannelAdapter { state: Arc<AppState>, app: AppHandle, pool: DbPool }`) and module declarations |
| `deps_impl.rs` | The single `impl rekindle_channel::ChannelMessagingDeps for ChannelAdapter` block — every method body delegates to one of the submodules below |
| `state_reads.rs` | Read paths: channel / thread / member lookups, permission computation |
| `state_mutations.rs` | Write paths into AppState (acquire lock, mutate, drop guard before await) |
| `dht.rs` | DHT writes / reads via `veilid_core` + `rekindle-records` |
| `persist.rs` | SQLite persists for messages / threads / sequences / slowmode state / retry queue |
| `events.rs` | `*Event` → tracing + `ChatEvent` / `CommunityEvent` local echo + delivery state emits |
| `misc.rs` | One-off helpers that do not fit the above |

The split exists because pre-Phase 23.D, every adapter was a single
`deps_impl.rs` that grew to 1 000+ LoC and tripped the no-god-module
rule. The Phase 14.r / Phase 23.D module-dir pattern caps each file
at 500 LoC and makes the structural choice ("is this a read or a
write or a DHT op?") explicit.

Adapters present in `services/`:

- `calls_adapter/` — Phase 14.q `rekindle_calls::signaling::CallRegistry`
- `channel_adapter/` — `rekindle_channel::ChannelMessagingDeps`
- `dm_adapter.rs` — `rekindle_dm::DmDeps` (single file, still under
  the cap)
- `files_adapter/` — `rekindle_files::FilesDeps`
- `governance_adapter/` — `rekindle_governance_runtime::Deps`
- `gossip_adapter/` — gossip mesh primitives
- `mek_adapter.rs` — `rekindle_mek_rotation::MekDistributeDeps`
- `presence_adapter/` — `rekindle_presence::PresenceDeps`
- `sync_adapter/` — `rekindle_sync::SyncDeps`
- `voice_adapter/` — voice transport / engine wiring
- `voice_signaling_adapter.rs` — `rekindle_calls::signaling::VoiceSignalingDeps`
- `video_adapter.rs` — video reassembly / frame routing

### 3. Pure-logic surfaces (`services/community/*.rs`, `services/dm/`, …)

These are the original v1 community / DM state machines that have
not yet (or will not) be hoisted into Tier-7 crates. They are
**Tauri-aware** (they import `crate::state::*`) but contain the
domain logic — gossip routing, channel-message dedup, raid
detection, automod evaluation, presence poll bucketing.

The boundary between "lives here" and "lives in a crate" is
gradual. The Phase-by-phase harvest plan is moving logic from
`services/community/*.rs` into Tier-7 crates one cohesive unit at a
time. Where the harvest is done (channel, DM, MEK rotation,
governance lifecycle, presence, friendship), `services/community/`
keeps a thin runtime + adapter that delegates to the crate. Where it
is not yet (raid detection, AutoMod regex matching, event reminder
scheduler, expression assets), the logic still lives in
`services/community/`.

## How to add a feature

When adding a new community feature (a new channel kind, a new
moderation primitive, a new presence sub-protocol), the choice of
where to put it follows a flowchart:

1. **Is the logic pure (no `AppState`, no `tauri::AppHandle`, no
   `veilid_core`)?** Put it in a Tier-7 crate
   (`rekindle-channel`, `rekindle-dm`, `rekindle-mek-rotation`,
   `rekindle-presence`, …). Define a `Deps` trait for the I/O
   surface.
2. **Is the logic Tauri-runtime glue (touches both `AppState` and
   the DHT/SQLite)?** Put it in a `*_runtime.rs` and have the body
   delegate to a pure-logic function.
3. **Is the logic the Schwarzschild bridge between the two?** Put it
   in a `*_adapter/` directory split as described above.

The fastest way to confirm the choice is to look at where the type
that owns the data lives:

- Type lives in a `rekindle-*` crate → put the logic there, define
  a `Deps` trait for I/O, implement the trait in `*_adapter/`.
- Type lives in `crate::state::*` → put the logic in `*_runtime.rs`.
- Both → write the runtime to compute the input, pass it to the
  crate function, write back through the adapter.

## Background services (not in any category)

A handful of services do not follow the runtime / adapter / pure-
logic split because they are long-running background tasks rather
than command-driven flows:

| Service | Path | Role |
|---|---|---|
| `dht_publish_service` | `services/dht_publish_service.rs` | Periodic profile re-publish |
| `game_service` | `services/game_service.rs` | Periodic game detection, publish to DHT |
| `game_publisher` | `services/game_publisher.rs` | Game detection → DHT subkey-4 publisher |
| `idle_service` | `services/idle_service.rs` | Auto-away on inactivity |
| `presence_service` | `services/presence_service.rs` | DHT `ValueChange` → presence update dispatch |
| `message_service/` | `services/message_service/` | Inbound friend `AppMessage` ingest, outbound envelope sign |
| `sync_service` | `services/sync_service.rs` | Pending message retry, unwatched-friend polling |
| `push_relay` | `services/push_relay.rs` | Mobile push relay client |

These all spawn `tokio::task::spawn` task handles at login (stored in
`AppState.background_handles` and aborted on logout) and have a
`shutdown_tx` companion in `AppState` that the graceful-shutdown
sequence triggers.

## Veilid dispatch loop

`services/veilid/lifecycle/dispatch.rs` is the central event router.
It receives `VeilidUpdate` variants and delegates by classifying:

- `AppMessage` → classified by content prefix
  - `b'V'` prefix → voice receive loop
  - Community envelopes → `services/community/gossip` (after dedup)
  - Otherwise → `services/message_service` (1:1 friend traffic)
- `AppCall` → DM invite handshake / community RPC handlers
- `ValueChange` → friend `presence_service` (profile records),
  `services/community/watch` (community records), or
  `services/cross_device_sync/watch` (personal sync records)
- `Attachment` → update `NodeHandle` state, emit `NetworkStatusEvent`
- `RouteChange` → re-allocate private routes via `routing_manager`

This is the one piece of `services/` that cannot move into the
runtime / adapter / pure-logic categorisation because it is the
inbound event source for everyone else.

## Why this matters

Before the runtime / adapter pattern, the same logic could live in
three different places: as a command handler in `commands/`, as a
service in `services/`, or as an inline closure inside the dispatch
loop. Reviewers spent time arguing about where new code should go
instead of reviewing the code itself.

The pattern collapses that to one decision: "does this code own
`AppState`?" Everything else falls out.

See [`decisions/0008-runtime-adapter-pattern.md`](../decisions/0008-runtime-adapter-pattern.md)
for the original argument that landed this pattern.
