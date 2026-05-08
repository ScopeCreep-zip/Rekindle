# System Architecture

Rekindle is a decentralized desktop chat and community application structured
as a four-layer stack. The frontend presents the user interface, Tauri bridges
it to the Rust backend, a tiered set of pure Rust crates implements all
business logic, and the Veilid network provides peer-to-peer transport and
distributed storage.

## Layer Stack

```
┌─────────────────────────────────────────────────────────┐
│                     SolidJS Frontend                    │
│  Windows, components, stores, handlers, styles          │
│  (src/)                                                 │
├─────────────────────────────────────────────────────────┤
│                   Tauri 2 IPC Bridge                    │
│  ~220 commands, 6 event channels, 7 windows             │
│  Plugin setup, system tray, window lifecycle            │
│  (src-tauri/)                                           │
├─────────────────────────────────────────────────────────┤
│                Pure Rust Crates (Tiers 1–7)             │
│  Tier 1   types          Tier 2  secrets                │
│  Tier 3   codec/records  Tier 4  route                  │
│  Tier 5   gossip         Tier 6  governance             │
│  Tier 7   dm/calls/files/video/link-preview             │
│  Plus:    protocol, crypto, voice, game-detect, sync    │
│  (crates/)                                              │
├─────────────────────────────────────────────────────────┤
│                    Veilid Network                       │
│  DHT storage (DFLT + SMPL), app_message routing         │
│  Private + safety routes, XChaCha20-Poly1305 transport  │
└─────────────────────────────────────────────────────────┘
```

Alongside the Tauri desktop app there is a parallel **daemon + CLI** track
(`rekindle-node` + `rekindle-cli`, both under `crates/`) that serves the same
Veilid protocol over an encrypted Noise-IK IPC bus. The Tauri shell does not
yet use the daemon — it links Veilid in-process via `rekindle-protocol` — but
both frontends speak the same wire format and SMPL governance. See `crates.md`
for how the daemon track factors `rekindle-transport` as the sole Veilid
boundary.

## Layer Responsibilities

| Layer | Responsibility |
|-------|---------------|
| SolidJS Frontend | Render state, forward user actions, no business logic |
| Tauri IPC Bridge | Route commands, manage windows/tray, emit events, host AppState |
| Pure Rust Crates | Protocol logic, cryptography, gossip mesh, governance CRDT, voice — zero Tauri dependency |
| Veilid Network | Peer discovery, message delivery, distributed storage, transport encryption |

## Crate Tiering (v2.0)

The Rust crates form a strict dependency hierarchy. Lower tiers know nothing
about higher tiers — they are pure logic with no I/O.

| Tier | Crate(s) | Role |
|------|----------|------|
| 1 | `rekindle-types` | Shared IDs, enums, error taxonomy. Zero deps on other rekindle crates. |
| 2 | `rekindle-secrets` | All key material (Ed25519, X25519, MEK). `Zeroize + ZeroizeOnDrop`. The sole crypto boundary. |
| 3 | `rekindle-codec`, `rekindle-records` | Signed envelope build/verify + DHT record lifecycle (SMPL schema, retry queue). |
| 4 | `rekindle-route` | Private route allocation, peer route cache, refresh lifecycle. |
| 5 | `rekindle-gossip` | Transport-agnostic gossip mesh: D-fanout selection, dedup, Lamport clocks, rate limiting. |
| 6 | `rekindle-governance` | Pure CRDT merge of `GovernanceEntry` variants. Reader-validates permissions. No I/O, no async. |
| 7 | `rekindle-dm`, `rekindle-calls`, `rekindle-files`, `rekindle-video`, `rekindle-link-preview` | Self-contained features built on lower tiers. |
| — | `rekindle-protocol`, `rekindle-crypto`, `rekindle-voice`, `rekindle-game-detect`, `rekindle-sync`, `rekindle-utils`, `rekindle-e2e-server` | Cross-cutting integration crates (Veilid plumbing, Signal sessions, audio pipeline, scanner, sync workers). |
| — | `rekindle-transport`, `rekindle-node`, `rekindle-cli` | Daemon/CLI track. `rekindle-transport` is the sole Veilid boundary on this path; `rekindle-node` is the daemon (Noise-IK IPC bus); `rekindle-cli` is the first IPC client. |

## Directory Tree

```
src/
├── main.tsx                          Entry point, path-based routing
├── windows/                          One component per Tauri window (7)
│   ├── LoginWindow.tsx
│   ├── BuddyListWindow.tsx
│   ├── ChatWindow.tsx
│   ├── DmWindow.tsx
│   ├── CommunityWindow.tsx
│   ├── SettingsWindow.tsx
│   └── ProfileWindow.tsx
├── components/                       Reusable UI components
│   ├── titlebar/                     Custom frameless titlebar
│   ├── buddy-list/                   Friend list, groups, DM tab, modals
│   ├── chat/                         Message list, bubbles, attachments, polls,
│   │                                 reactions, voice messages, threads
│   ├── community/                    Channels, members, settings tabs, events,
│   │                                 forum view, stage panel, onboarding wizard
│   │   └── settings/                 Overview/Members/Roles/Bans/Invites/Channels/
│   │                                 AutoMod/AuditLog/Security tabs
│   ├── voice/                        Voice panel, participants
│   ├── status/                       Status picker, dot, network indicator
│   ├── settings/                     Relay, push relay sections
│   └── common/                       Avatar, modal, context menu, scroll area, toast
├── stores/                           SolidJS reactive state (12 stores)
├── ipc/
│   ├── commands.ts                   Typed invoke() wrappers (~220 commands)
│   ├── channels.ts                   Event subscriptions (listen)
│   ├── invoke.ts                     Conditional invoke (Tauri / E2E HTTP)
│   ├── hydrate.ts                    State hydration on login
│   ├── avatar.ts                     Avatar data handling
│   └── permissions.ts                Permission bitmask helpers
├── handlers/                         Named event-handler functions (13 files)
├── hooks/                            Reusable composables (createContextMenu)
├── utils/                            Formatting, time, color, masking, permissions
├── styles/                           Global CSS (Tailwind @apply)
└── icons.ts                          Icon definitions

src-tauri/
├── src/
│   ├── lib.rs                        App entry, plugin registration, command registry
│   ├── main.rs                       Desktop entry point
│   ├── state.rs                      AppState, SharedState, type definitions (40+ fields)
│   ├── state_helpers.rs              Read-only state accessors
│   ├── db.rs                         SQLite pool, schema versioning (SCHEMA_VERSION = 56)
│   ├── db_helpers.rs                 db_call / db_call_or_default / db_fire helpers
│   ├── friend_repo.rs                Friend list CRUD
│   ├── channel_repo.rs               Community channel CRUD
│   ├── message_repo.rs               Message persistence and queries
│   ├── invite_helpers.rs             Outgoing invite tracking
│   ├── deep_links.rs                 rekindle:// URI handler
│   ├── serde_helpers.rs              Custom serde adapters
│   ├── keystore.rs                   iota_stronghold wrapper (per-identity files)
│   ├── tray.rs                       System tray setup
│   ├── windows.rs                    Window creation helpers
│   ├── commands/                     IPC command modules
│   │   ├── auth.rs (6)               create_identity, login, logout, list/delete
│   │   ├── chat.rs (5)               send_message, typing, history, mark_read,
│   │   │                             prepare_chat_session
│   │   ├── friends.rs (17)           add/remove/accept/reject, groups, invites,
│   │   │                             block/unblock, cancel_request, presence
│   │   ├── dm.rs (6)                 list/start/accept/decline/send/get
│   │   ├── community/ (30 modules)   See Community Commands section
│   │   ├── voice.rs (12)             join/leave, mute/deafen, devices, stage,
│   │   │                             server_mute_member
│   │   ├── status.rs (5)             status, nickname, avatar, status_message
│   │   ├── game.rs (3)               get_game_status, get_game_name, launch_game_to_server
│   │   ├── relay.rs (4)              volunteer_relay, revoke_relay, list_*
│   │   ├── search.rs (1)             search_messages
│   │   ├── sync.rs (10)              ensure_personal_sync_record, pairing,
│   │   │                             read/write manifest, read_state, prefs, devices
│   │   ├── push_relay.rs (3)         register/unregister/list with push relay
│   │   ├── settings.rs (3)           get/set preferences, check_for_updates
│   │   └── window.rs (7)             show_buddy_list, open_*_window, get_network_status
│   ├── channels/                     Event type definitions
│   │   ├── chat_channel.rs           ChatEvent (10 variants)
│   │   ├── presence_channel.rs       PresenceEvent (4 variants)
│   │   ├── voice_channel.rs          VoiceEvent (6 variants)
│   │   ├── notification_channel.rs   NotificationEvent, NetworkStatusEvent
│   │   └── community_channel.rs      CommunityEvent (50+ variants)
│   └── services/                     Background services
│       ├── veilid/                   Node lifecycle, dispatch loop, control events
│       │   └── lifecycle/            cleanup, dispatch, node, route_refresh, status
│       ├── voice/                    Send loop, receive loop, MCU loop, signaling
│       ├── community/                Gossip, governance, presence, channel messages,
│       │                             threads, polls, reactions, video, expressions,
│       │                             files, link previews, automod, raid_detection,
│       │                             segments, stage, mek_rotation, …
│       │   ├── join/                 bootstrap, flow, helpers, history, rejoin, state
│       │   ├── presence/             poll, registry, sync
│       │   └── analytics/            buckets, growth, channel/member metrics
│       ├── cross_device_sync/        Pairing, record sync, subkey I/O, merge, watch
│       ├── relay/                    Strand Relay forwarding, presence, offer
│       ├── dm/                       accept, create, ingest, messages, store
│       ├── search/                   context, query, dm, threads, messages
│       ├── dht_publish_service.rs    Periodic DHT republish
│       ├── game_service.rs           Game detection scan loop
│       ├── idle_service.rs           Auto-away on inactivity
│       ├── presence_service.rs       Presence heartbeat
│       ├── message_service.rs        Envelope sign/dispatch
│       ├── sync_service.rs           Pending message retry
│       └── push_relay.rs             Mobile push relay client
├── migrations/
│   └── 001_init.sql                  SQLite schema (single source of truth)
└── Cargo.toml

crates/                               (22 workspace members — see crates.md)

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
| Events | Rust → Frontend | `app.emit()` / `listen()` | Incoming messages, presence updates, typing indicators |

Commands are synchronous request-response calls. Events are push-based
notifications emitted by background services whenever state changes.

## Window Architecture

Only the `login` window is declared statically in `tauri.conf.json`. All other
windows are created at runtime by helpers in `src-tauri/src/windows.rs`. Each
window has its own URL path. The SolidJS `Switch` component in `main.tsx`
reads `window.location.pathname` and renders the corresponding window component.

| Window | Label | Path | Notes |
|--------|-------|------|-------|
| Login | `login` | `/login` | 380 x 480, declared in `tauri.conf.json` |
| Buddy List | `buddy-list` | `/buddy-list` | Narrow vertical (320 x 650), hides to tray on close |
| Chat | `chat-{pubkey prefix}` | `/chat?peer={key}` | One per 1:1 conversation |
| DM | `dm-{record-key prefix}` | `/dm?record={key}` | One per DM / group DM |
| Community | `community-{id}` | `/community?id={id}` | One per joined community |
| Settings | `settings` | `/settings` | Single instance |
| Profile | `profile-{key prefix}` | `/profile?key={key}` | One per peer |

All windows use `decorations: false` and `transparent: true` for the frameless
Xfire-style appearance. Closing the login window when no buddy list is visible
exits the app; the buddy list itself is hidden to the system tray on close.

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
                   ┌──────────────┐   emit("chat-event")   ┌──────────┐
                   │   Tauri      │ ─────────────────────→ │ Frontend │
                   │   app.emit() │                        │  (store) │
                   └──────────────┘                        └──────────┘
```

## Data Flow: Friend Comes Online

```
┌──────────────┐  VeilidUpdate::ValueChange  ┌────────────────┐
│  Veilid DHT  │ ────────────────────────→   │ veilid::       │
│  (watched    │                             │ dispatch       │
│   record)    │                             └───────┬────────┘
└──────────────┘                                     │
                                                     ▼
                                              ┌────────────────┐
                                              │ presence_      │
                                              │ service        │
                                              │ (update state) │
                                              └───────┬────────┘
                                                      │
                          ┌───────────────────────────┤
                          ▼                           ▼
                   ┌──────────────┐        ┌──────────────────┐
                   │  AppState    │        │ emit("presence-  │
                   │  .friends    │        │       event")    │
                   │  (update)    │        └────────┬─────────┘
                   └──────────────┘                 │
                                                    ▼
                                             ┌──────────────┐
                                             │  Frontend    │
                                             │  friends     │
                                             │  store       │
                                             └──────────────┘
```

## Data Flow: Community Channel Message (v2.0)

Community messages travel three paths in parallel for durability and speed.
For the full chiral-network architecture — universal SMPL schema, CRDT
governance, MEK rotation, plate-gate scaling, design principles — see
[`communities.md`](communities.md) (in this directory).

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

The gossip path (Tier 5) gives sub-second delivery to online peers. The SMPL
write (Tier 3) is the durable record that offline peers fetch on next login.
The watch/inspect path (services/community/watch.rs + presence/poll.rs) reconciles
late joiners and detects gaps via per-sender sequence numbers.

## Plate-Gate Scaling and Cross-Segment Routing (v2.0)

A single SMPL DHT record stores at most 255 member subkeys (Veilid practical
limit). Communities larger than 255 members are split into **fractal
segments** — additional registry + governance records announced via
`GovernanceEntry::SegmentAdded`. Each segment is its own
255-slot SMPL record; the protocol unifies them at the data-merge layer
(architecture spec §15, ds-aligned v2 plan §16).

The CRDT model is an **ORMap-of-CRDTs** in Shapiro/Almeida terminology
(Shapiro 2011 *Conflict-Free Replicated Data Types*; Almeida et al. 2016
*Delta State Replicated Data Types* arXiv:1603.01529). Each segment is
its own join-semilattice; the community state is the product CRDT under
coordinate-wise join. **Cross-segment invariants are reader-validated,
never written into per-segment state** — every peer fetches each
segment's author entries and runs the same `rekindle_governance::merge`
function over the union.

**What we ship today (C1 phase):**

| Concern               | Mechanism                                                                                               |
|-----------------------|---------------------------------------------------------------------------------------------------------|
| Membership discovery  | `services/community/segments.rs::segment_descriptors` lists every active segment (registry + governance keys) from merged governance state. The presence poll iterates all segments and aggregates. |
| Admin expansion       | `expand_community_segment` (`commands/community/segments.rs`) — admin-only, writes a `SegmentAdded` governance entry that creates the new SMPL records. |
| Slot claim            | `services/community/join/flow.rs` walks segment descriptors in order, claims the first free slot in any segment. |
| Governance fetch      | `commands/auth.rs::rebuild_governance_from_dht` does a two-pass merge: primary segment first, then every additional segment listed in `gov_state_v1.segments`. CRDT idempotence makes the second pass safe. |
| Gossip                | Crosses segment boundaries naturally — gossip is keyed by `(community_id, channel_id)`, not by segment. All peers participate in the same epidemic broadcast graph regardless of which segment hosts their slot. |
| Hard cap              | `MAX_SEGMENTS = 8` (≈2040 members) — soft cap; raising the constant lifts the limit, but read amplification on presence poll grows linearly. |

**Cross-segment messaging (the routing question):**

Channel records are themselves segmented at scale. The architecture spec
(§15.4) marks them as **lazy** — created only when the first member of a
new segment writes to that channel. We have *not* shipped lazy channel
record creation in C1; it is deferred to **C1-2** along with cross-segment
MEK distribution and the `ChannelSegmentLinked` governance entry that
announces a new segment-scoped channel record key.

Until C1-2 lands, all messages flow through the segment-0 channel record
plus the gossip mesh. This is correct for any community that fits in
segment 0 (≤255 members). Communities that have expanded past one
segment will see two behaviours:

1. **Online recipients:** Gossip carries every message regardless of
   sender or recipient segment, so live conversations work across all
   segments.
2. **Offline recipients in segments ≥1:** They will not pick up
   messages on next login until C1-2 introduces per-segment channel
   records. Until then, segment expansion is exposed as a UX nudge in
   the presence poll (`SegmentExpansionAvailable`); the community is
   technically usable but admins should treat segments ≥1 as
   "real-time-only" and avoid relying on offline catch-up for those
   members.

**Why deferred, not workarounded:** the alternative — bridging messages
through a designated segment-0 relay peer — would reintroduce a
single-point-of-failure of exactly the kind v2.0 was built to remove
(architecture spec §1.4). Lazy per-segment channel records are the
spec-mandated route, and shipping a relay-bridge stopgap would make it
harder to land the real solution. C1-2 keeps the membership-discovery
and admin-expansion work shipped today usable as soon as channel-record
fan-out lands, with no migration step.

**External references:**

- Shapiro et al. 2011 *Conflict-Free Replicated Data Types* — ORMap-of-CRDTs.
- Almeida et al. 2016 *Delta State Replicated Data Types* — arXiv:1603.01529.
- Riak DT — production reference for ORMap-of-CRDTs.
- Matrix faster-joins — lazy member hydration pattern (informs the C1-2
  lazy channel-record design).
- Discord guild sharding — confirms even centralised systems give up on
  "all members at once" past a few thousand.
