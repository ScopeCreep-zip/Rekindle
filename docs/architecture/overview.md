# System Architecture

Rekindle is a decentralised desktop chat and community application
structured as a four-layer stack. The frontend presents the user
interface, Tauri bridges it to the Rust backend, a tiered set of pure
Rust crates implements all business logic, and the Veilid network
provides peer-to-peer transport and distributed storage.

## Layer Stack

```
┌─────────────────────────────────────────────────────────┐
│                     SolidJS Frontend                    │
│  8 windows, components, stores, handlers, styles        │
│  (src/)                                                 │
├─────────────────────────────────────────────────────────┤
│                   Tauri 2 IPC Bridge                    │
│  ~241 commands, 5 event channels, plugin setup,         │
│  system tray, window lifecycle, single-source event     │
│  dispatch (Phase 23.A)                                  │
│  (src-tauri/)                                           │
├─────────────────────────────────────────────────────────┤
│                Pure Rust Crates (33 members)            │
│  Tier 1   types                                         │
│  Tier 2   secrets, vault, audit                         │
│  Tier 3   codec, records, events, idempotency,          │
│           mek-rotation, analytics                       │
│  Tier 4   route, lifecycle                              │
│  Tier 5   gossip, presence, friendship                  │
│  Tier 6   governance + governance-runtime               │
│  Tier 7   channel, dm, calls, files, video, link-       │
│           preview                                       │
│  Plus     protocol, transport, crypto, voice, sync,     │
│           game-detect, utils, e2e-server, node, cli     │
│  (crates/)                                              │
├─────────────────────────────────────────────────────────┤
│                    Veilid Network                       │
│  DHT storage (DFLT + SMPL), app_message routing         │
│  Private + safety routes, XChaCha20-Poly1305 transport  │
└─────────────────────────────────────────────────────────┘
```

Two frontends sit on top of the crate stack:

- **Desktop app** (`src-tauri/`) — links `rekindle-protocol` directly and
  runs the Veilid node in-process. This is the primary build today.
- **Daemon + CLI** (`rekindle-node` + `rekindle-cli` over
  `rekindle-transport`) — long-running daemon owns the Veilid node and
  serves clients over a Noise-IK encrypted IPC bus.

Both speak the same wire format and SMPL governance — the difference is
process layout, not protocol. See
[`decisions/0001-veilid-as-transport.md`](../decisions/0001-veilid-as-transport.md)
for the dual-boundary rationale, and
[`crates.md`](crates.md) for how each Veilid boundary stays isolated.

## Layer Responsibilities

| Layer | Responsibility |
|-------|---------------|
| SolidJS Frontend | Render state, forward user actions, no business logic |
| Tauri IPC Bridge | Route commands, manage windows / tray, emit events, host AppState |
| Pure Rust Crates | Protocol logic, cryptography, gossip mesh, governance CRDT, voice — zero Tauri dependency, parameterised over `Deps` traits |
| Veilid Network | Peer discovery, message delivery, distributed storage, transport encryption |

## Crate Tiering

The Rust crates form a strict dependency hierarchy. Lower tiers know
nothing about higher tiers — they are pure logic with no I/O. The full
inventory is [`crates.md`](crates.md); per-crate detail is
[`crates-tier-detail.md`](crates-tier-detail.md). Summary:

| Tier | Crate(s) | Role |
|------|----------|------|
| 1 | `rekindle-types` | Shared IDs, enums, error taxonomy. Zero deps on other rekindle crates. |
| 2 | `rekindle-secrets`, `rekindle-vault`, `rekindle-audit` | All key material (`secrets`), SQLCipher persistent store (`vault`), tamper-evident BLAKE3 keyed hash chain (`audit`). The sole crypto / persistence boundary. |
| 3 | `rekindle-codec`, `rekindle-records`, `rekindle-events`, `rekindle-idempotency`, `rekindle-mek-rotation`, `rekindle-analytics` | Signed envelope build / verify, DHT record lifecycle, event dedup + journal, command idempotency, MEK cascade rotation, local-only analytics. |
| 4 | `rekindle-route`, `rekindle-lifecycle` | Private route allocation + cache; 9-state app FSM + `TransportGuard`. |
| 5 | `rekindle-gossip`, `rekindle-presence`, `rekindle-friendship` | Transport-agnostic gossip primitives; presence orchestrators; inbox-scan coordinator. |
| 6 | `rekindle-governance`, `rekindle-governance-runtime` | Pure CRDT merge (no I/O, no async); async lifecycle layer for community origin / bootstrap / join / segments. |
| 7 | `rekindle-channel`, `rekindle-dm`, `rekindle-calls`, `rekindle-files`, `rekindle-video`, `rekindle-link-preview` | Self-contained features built on lower tiers. |
| — | `rekindle-protocol`, `rekindle-transport`, `rekindle-crypto`, `rekindle-voice`, `rekindle-game-detect`, `rekindle-sync`, `rekindle-utils`, `rekindle-e2e-server` | Cross-cutting integration. `protocol` is the **desktop** Veilid boundary; `transport` is the **daemon** Veilid boundary. |
| — | `rekindle-node`, `rekindle-cli` | Daemon process + CLI/TUI client. Communicate over Noise-IK IPC bus. |

Eleven of these crates are recent harvest extractions; see
[`decisions/0009-crate-harvest-tiers.md`](../decisions/0009-crate-harvest-tiers.md).

## Directory Tree

```
src/
├── main.tsx                          Entry point, path-based routing
├── windows/                          One component per Tauri window (8)
│   ├── LoginWindow.tsx
│   ├── BuddyListWindow.tsx
│   ├── ChatWindow.tsx
│   ├── DmWindow.tsx
│   ├── CommunityWindow.tsx
│   ├── SettingsWindow.tsx
│   ├── ProfileWindow.tsx
│   └── CallWindow.tsx
├── components/                       Reusable UI components by feature
│   ├── titlebar/                     Custom frameless titlebar
│   ├── buddy-list/                   Friend list, groups, DM tab, modals
│   │                                 (incl. StartGroupCallModal)
│   ├── chat/                         Message list, bubbles, attachments,
│   │                                 polls, reactions, voice messages,
│   │                                 threads, ChannelChat, SearchPanel
│   ├── community/                    Channels, members, settings tabs,
│   │                                 events, forum view, stage, onboarding,
│   │                                 CreateThreadModal, JoinProgressStepper
│   │   └── settings/                 Overview / Members / Roles / Bans /
│   │                                 Invites / Channels / AutoMod / AuditLog /
│   │                                 Analytics / Security tabs
│   ├── voice/                        Voice + video call surfaces — 16 files
│   │                                 incl. CallController, ActiveCallPanel,
│   │                                 VideoCallPanel, GroupCallPanel,
│   │                                 SoundboardPanel, plus call_stage/
│   ├── status/                       Status picker, dot, network indicator
│   ├── settings/                     Relay, push relay sections
│   └── common/                       Avatar, modal, scroll area, toast,
│                                     AnnounceRegion
├── stores/                           SolidJS reactive state (16 stores —
│                                     incl. join, lifecycle)
├── ipc/
│   ├── commands.ts                   Typed invoke() wrappers (~241 commands)
│   ├── channels.ts                   Event subscriptions (listen)
│   ├── commands/                     Per-domain command modules
│   │                                 (account, community, governance,
│   │                                 sync, system, voice, types)
│   ├── channels/                     Per-domain channel modules
│   ├── invoke.ts                     Conditional invoke (Tauri / E2E HTTP)
│   ├── hydrate.ts                    State hydration on login
│   ├── avatar.ts                     Avatar data handling
│   └── permissions.ts                Permission bitmask helpers
├── handlers/                         Named event-handler functions —
│   │                                 30+ files at top level
│   ├── community/                    Community dispatcher + per-concern
│   │                                 handlers (channels, moderation, roles,
│   │                                 invites, lifecycle, profile, shared)
│   └── calls/                        Call actions, events, ring
├── hooks/                            Reusable composables
├── utils/                            Formatting, time, color, masking
├── styles/                           Global CSS (Tailwind @apply)
└── icons.ts                          Icon definitions

src-tauri/
├── src/
│   ├── lib.rs                        App entry, plugin registration,
│   │                                 #![recursion_limit = "512"]
│   ├── main.rs                       Desktop entry point
│   ├── invoke.rs                     tauri::generate_handler! — ~241 cmds
│   ├── setup.rs                      Tauri .setup() — Veilid init, DB,
│   │                                 event loop spawn
│   ├── state.rs                      Module re-export
│   ├── state/                        AppState (~47 fields) split by concern
│   │   ├── app_state.rs              Field definitions
│   │   ├── circuit.rs                Per-community circuit-breaker types
│   │   ├── community.rs              CommunityState
│   │   ├── friend.rs                 FriendState
│   │   ├── gossip.rs                 Gossip dedup + rate-limit types
│   │   └── runtime.rs                Runtime handles / shutdown signals
│   ├── state_helpers/                Read-only accessors by concern
│   │                                 (circuit_breaker, communities,
│   │                                 dht_records, friends, governance,
│   │                                 governance_persist, identity, node,
│   │                                 routes)
│   ├── db.rs                         SQLite pool, SCHEMA_VERSION = 71
│   ├── db_helpers.rs                 db_call / db_call_or_default / db_fire
│   ├── event_dispatch.rs             Phase 23.A — single-source emit router
│   ├── keystore/                     VaultStore-backed (no iota_stronghold)
│   │   ├── signal.rs                 Signal identity / sessions / prekeys / PQ
│   │   ├── community_keys.rs         Community MEK, slot / registry keypairs
│   │   ├── channel_mek.rs            Per-channel + per-generation MEK
│   │   └── audit.rs                  Audit MAC key + tail anchor
│   ├── audit_repo/                   Audit chain persistence (chain, store)
│   ├── audit_view.rs                 Audit log query / export
│   ├── channel_materialize.rs        Channel state materialisation
│   ├── channel_repo.rs               Community channel CRUD
│   ├── community_loader/             Community state restore on startup
│   ├── envelope_store_sqlite.rs      Durable pending-envelope queue
│   ├── friend_store_sqlite.rs        Receive-path friend authority
│   ├── friend_repo.rs                Friend list CRUD
│   ├── message_repo.rs               Message persistence
│   ├── message_view.rs               Message read views
│   ├── signal_stores.rs              Signal store DB-level access
│   ├── invite_helpers.rs             Outgoing invite tracking
│   ├── serde_helpers.rs              Custom serde adapters
│   ├── deep_links.rs                 rekindle:// URI handler
│   ├── platform.rs                   Per-OS setup glue
│   ├── shutdown.rs                   Exit-event cleanup
│   ├── shortcuts.rs                  Global hotkey registration
│   ├── tray.rs                       System tray setup
│   ├── video_channels.rs             High-throughput video channel registry
│   ├── windows.rs                    Window creation helpers
│   ├── commands/                     IPC command modules (49 .rs files;
│   │                                 community/ has 32 submodules)
│   ├── channels/                     Event type definitions (chat,
│   │                                 community, notification, presence,
│   │                                 voice)
│   └── services/                     Background services (~229 .rs files,
│                                     runtime / adapter / pure-logic
│                                     layering — see services-pattern.md)
├── migrations/
│   └── 001_init.sql                  SQLite schema (single source of truth)
└── Cargo.toml

crates/                               (33 workspace members — see crates.md)

schemas/                              Cap'n Proto schema definitions
├── account.capnp                     AccountHeader, ContactEntry, ChatEntry
├── community.capnp                   Community, Channel, Role, PermissionOverwrite
├── conversation.capnp                ConversationHeader
├── friend.capnp                      FriendRequest, FriendList, FriendEntry
├── identity.capnp                    UserProfile, PreKeyBundle
├── message.capnp                     MessageEnvelope, ChatMessage, Attachment
├── presence.capnp                    PresenceUpdate, GameStatus
└── voice.capnp                       VoiceSignaling
```

## IPC Patterns

| Pattern | Direction | Mechanism | Use Cases |
|---------|-----------|-----------|-----------|
| Commands | Frontend → Rust | `invoke()` / `#[tauri::command]` | Login, send message, add friend, change status |
| Events | Rust → Frontend | `app.emit()` via `event_dispatch.rs` → `listen()` | Incoming messages, presence updates, typing indicators |

Commands are synchronous request-response calls. Events are push-based
notifications produced by background services. **All `app.emit()` calls
flow through `src-tauri/src/event_dispatch.rs`** (Phase 23.A single-
source router) so that cross-cutting concerns — rate limiting,
journalling, replay-on-reconnect — live in one place. See
[`event-dispatch.md`](event-dispatch.md).

## Window Architecture

Only the `login` window is declared statically in `tauri.conf.json`.
All other windows are created at runtime by helpers in
`src-tauri/src/windows.rs`. Each window has its own URL path. The
SolidJS `Switch` component in `main.tsx` reads
`window.location.pathname` and renders the corresponding window
component.

| Window | Label | Path | Notes |
|--------|-------|------|-------|
| Login | `login` | `/login` | 380 × 480, declared in `tauri.conf.json` |
| Buddy List | `buddy-list` | `/buddy-list` | Narrow vertical (320 × 650), hides to tray on close |
| Chat | `chat-{pubkey prefix}` | `/chat?peer={key}` | One per 1:1 conversation |
| DM | `dm-{record-key prefix}` | `/dm?record={key}` | One per DM / group DM |
| Community | `community-{id}` | `/community?id={id}` | One per joined community |
| Settings | `settings` | `/settings` | Single instance |
| Profile | `profile-{key prefix}` | `/profile?key={key}` | One per peer |
| Call | `call-{id}` | `/call?id={id}` | Active call surface (Phase 14.q) |

All windows use `decorations: false` and `transparent: true` for the
frameless Xfire-style appearance. Closing the login window when no
buddy list is visible exits the app; the buddy list itself is hidden
to the system tray on close.

## Data Flow: Sending a 1:1 Message

```
┌──────────┐    invoke()     ┌──────────┐   Signal encrypt   ┌────────────────┐
│ Frontend │ ──────────────→ │  Tauri   │ ────────────────→  │ rekindle-crypto│
│ MessageInput│ send_message │ commands │                    │   (encrypt)    │
└──────────┘                 └────┬─────┘                    └──────┬─────────┘
                                  │                                  │
                                  │ ciphertext                       │
                                  ▼                                  ▼
                            ┌──────────────┐  build & sign    ┌─────────────┐
                            │ message_     │ ───────────────→ │ rekindle-   │
                            │ service      │  MessageEnvelope │ codec       │
                            └──────┬───────┘                  └──────┬──────┘
                                   │ app_message(route_id, bytes)    │
                                   ▼                                  │
                            ┌──────────────┐                         │
                            │   Veilid     │ ←───────────────────────┘
                            │   Network    │
                            └──────────────┘
```

## Data Flow: Receiving a 1:1 Message

```
┌──────────────┐  VeilidUpdate::AppMessage  ┌──────────────┐
│   Veilid     │ ────────────────────────→  │ veilid::     │
│   Network    │                            │ dispatch     │
└──────────────┘                            └──────┬───────┘
                                                   │ classify by prefix
                          ┌────────────────────────┼─────────────────────┐
                          ▼                        ▼                     ▼
                   ┌──────────────┐         ┌──────────────┐      ┌──────────────┐
                   │ message_     │         │ community::  │      │ voice receive│
                   │ service      │         │ gossip       │      │ loop         │
                   │ (1:1 + DM)   │         │ (community)  │      │ (voice 'V')  │
                   └──────┬───────┘         └──────────────┘      └──────────────┘
                          │
                          ▼
                   ┌──────────────┐         ┌──────────┐
                   │rekindle-crypto│  +     │  SQLite  │
                   │  (decrypt)   │         │  (store) │
                   └──────────────┘         └──────────┘
                          │
                          │ plaintext
                          ▼
                   ┌──────────────┐   event_dispatch::emit_journaled  ┌──────────┐
                   │   Tauri      │ ────────────────────────────────→ │ Frontend │
                   │ EmitEnvelope │   ("chat-event")                  │  (store) │
                   └──────────────┘                                   └──────────┘
```

## Data Flow: Community Channel Message

Community messages travel three paths in parallel for durability and
speed: SMPL channel record write (durability), gossip mesh broadcast
(fanout-D speed), and watch / inspect subscribers (catch-up for late
joiners). The full chiral-network architecture — universal SMPL schema,
CRDT governance, MEK rotation, plate-gate scaling — lives in
[`communities-overview.md`](communities-overview.md),
[`communities-channels.md`](communities-channels.md), and
[`communities-governance.md`](communities-governance.md).

```
            ┌──────────────────────────────────────────────────┐
            │                  Sender                          │
            │  send_channel_message → MEK encrypt → envelope   │
            └────────┬─────────────────────────┬──────────────┘
                     │                         │
       ┌─────────────┴────────┐  ┌─────────────┴────────────────┐
       ▼                      ▼  ▼                              ▼
┌─────────────┐   ┌─────────────────┐                ┌────────────────────┐
│ SMPL channel│   │ Gossip mesh     │                │ Watch / inspect    │
│ record write│   │ broadcast       │                │ subscribers        │
│ (durability)│   │ (D-fanout, fast)│                │ (catch-up via DHT) │
└─────────────┘   └─────────────────┘                └────────────────────┘
       │                      │                              │
       └─────────────────────┬┴──────────────────────────────┘
                             ▼
                ┌─────────────────────────────┐
                │ Receiver: dedup + Lamport   │
                │ ordering + governance gate  │
                │ → SQLite + chat-event emit  │
                └─────────────────────────────┘
```

The gossip path (Tier 5) gives sub-second delivery to online peers. The
SMPL write (Tier 3) is the durable record that offline peers fetch on
next login. The watch / inspect path
(`services/community/watch.rs` + `presence/poll.rs`) reconciles late
joiners and detects gaps via per-sender sequence numbers.

## Plate-Gate Scaling

A single SMPL DHT record stores at most 255 member subkeys (Veilid
practical limit). Communities larger than 255 members split into
**fractal segments** — additional registry + governance records
announced via `GovernanceEntry::SegmentAdded`. Each segment is its own
255-slot SMPL record; the protocol unifies them at the data-merge
layer (architecture spec §15, ds-aligned v2 plan §16).

The CRDT model is an **ORMap-of-CRDTs** in Shapiro/Almeida terminology
(Shapiro 2011 *Conflict-Free Replicated Data Types*; Almeida et al.
2016 *Delta State Replicated Data Types* arXiv:1603.01529). Each
segment is its own join-semilattice; the community state is the
product CRDT under coordinate-wise join. **Cross-segment invariants
are reader-validated, never written into per-segment state** — every
peer fetches each segment's author entries and runs the same
`rekindle_governance::merge` function over the union.

For the full segmentation flow, lazy channel-record creation roadmap,
and cross-segment messaging design, see
[`communities-governance.md`](communities-governance.md).
