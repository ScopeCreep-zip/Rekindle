# System Architecture

Rekindle is a decentralized desktop chat application structured as a four-layer
stack. The frontend presents the user interface, Tauri bridges it to the Rust
backend, pure Rust crates implement all business logic, and the Veilid network
provides peer-to-peer transport and distributed storage.

## Layer Stack

```
┌─────────────────────────────────────────────────────────┐
│                     SolidJS Frontend                    │
│  Windows, components, stores, handlers, styles          │
│  (src/)                                                 │
├─────────────────────────────────────────────────────────┤
│                   Tauri 2 IPC Bridge                    │
│  Commands (Frontend→Rust), Events (Rust→Frontend)       │
│  Window management, system tray, plugins                │
│  (src-tauri/)                                           │
├─────────────────────────────────────────────────────────┤
│                   Pure Rust Crates                      │
│  rekindle-protocol   rekindle-crypto                    │
│  rekindle-game-detect   rekindle-voice                  │
│  (crates/)                                              │
├─────────────────────────────────────────────────────────┤
│                    Veilid Network                       │
│  DHT storage, app_message routing, private routes       │
│  XChaCha20-Poly1305 transport encryption                │
└─────────────────────────────────────────────────────────┘
```

## Layer Responsibilities

| Layer | Responsibility |
|-------|---------------|
| SolidJS Frontend | Render state, forward user actions, no business logic |
| Tauri IPC Bridge | Route commands, manage windows/tray, emit events, manage app state |
| Pure Rust Crates | Protocol logic, cryptography, game detection, voice — zero Tauri dependency |
| Veilid Network | Peer discovery, message delivery, distributed storage, transport encryption |

## Directory Tree

```
src/
├── main.tsx                          Entry point, path-based routing
├── windows/                          One component per Tauri window
│   ├── LoginWindow.tsx
│   ├── BuddyListWindow.tsx
│   ├── ChatWindow.tsx
│   ├── CommunityWindow.tsx
│   ├── SettingsWindow.tsx
│   └── ProfileWindow.tsx
├── components/                       Reusable UI components
│   ├── titlebar/
│   ├── buddy-list/
│   ├── chat/
│   ├── community/
│   ├── voice/
│   ├── status/
│   └── common/
├── stores/                           SolidJS reactive state
├── ipc/                              Tauri IPC wrappers
│   ├── commands.ts                   Typed invoke() wrappers
│   ├── channels.ts                   Event subscriptions (listen)
│   ├── invoke.ts                     Conditional invoke (Tauri / E2E HTTP)
│   ├── hydrate.ts                    State hydration on login
│   └── avatar.ts                     Avatar data handling
├── handlers/                         Named event handler functions
├── styles/                           Global CSS (Tailwind @apply)
└── icons.ts                          Icon definitions

src-tauri/
├── src/
│   ├── lib.rs                        App entry, plugin registration, setup
│   ├── main.rs                       Desktop entry point
│   ├── state.rs                      AppState, SharedState, type definitions
│   ├── db.rs                         SQLite pool, schema versioning
│   ├── keystore.rs                   iota_stronghold wrapper
│   ├── tray.rs                       System tray setup
│   ├── windows.rs                    Window creation helpers
│   ├── commands/                     IPC command modules
│   │   ├── auth.rs                   create_identity, login, logout, etc.
│   │   ├── chat.rs                   send_message, get_history, mark_read
│   │   ├── friends.rs                add/remove/accept/reject, groups
│   │   ├── community.rs              create, join, channels, members
│   │   ├── voice.rs                  join/leave channel, mute/deafen
│   │   ├── status.rs                 set_status, nickname, avatar
│   │   ├── game.rs                   get_game_status
│   │   ├── settings.rs              get/set preferences, check updates
│   │   └── window.rs                 show_buddy_list, open_chat, etc.
│   ├── channels/                     Event type definitions
│   │   ├── chat_channel.rs           ChatEvent enum
│   │   ├── presence_channel.rs       PresenceEvent enum
│   │   ├── voice_channel.rs          VoiceEvent enum
│   │   └── notification_channel.rs   NotificationEvent, NetworkStatusEvent
│   └── services/                     Background services
│       ├── veilid_service.rs         Node lifecycle, dispatch loop
│       ├── message_service.rs        Incoming message processing
│       ├── presence_service.rs       DHT presence watching
│       ├── sync_service.rs           Offline message retry
│       ├── community_service.rs      Community DHT sync
│       └── game_service.rs           Game detection loop
├── migrations/
│   └── 001_init.sql                  SQLite schema
└── Cargo.toml

crates/
├── rekindle-protocol/src/            Veilid networking, DHT, serialization
├── rekindle-crypto/src/              Ed25519, Signal Protocol, group encryption
├── rekindle-game-detect/src/         Process scanning, game database
└── rekindle-voice/src/               Opus codec, audio I/O, VAD, transport

schemas/                              Cap'n Proto schema definitions
├── message.capnp
├── identity.capnp
├── presence.capnp
├── community.capnp
├── friend.capnp
├── voice.capnp
├── conversation.capnp
└── account.capnp
```

## IPC Patterns

| Pattern | Direction | Mechanism | Use Cases |
|---------|-----------|-----------|-----------|
| Commands | Frontend → Rust | `invoke()` / `#[tauri::command]` | Login, send message, add friend, change status |
| Events | Rust → Frontend | `app.emit()` / `listen()` | Incoming messages, presence updates, typing indicators |

Commands are synchronous request-response calls. Events are push-based
notifications emitted by background services whenever state changes.

## Window Architecture

Each window type maps to a separate Tauri webview with its own URL path. The
SolidJS `Switch` component in `main.tsx` reads `window.location.pathname` and
renders the corresponding window component.

| Window | Label | Path | Dimensions |
|--------|-------|------|------------|
| Login | `login` | `/login` | 380 x 480 |
| Buddy List | `buddy-list` | `/buddy-list` | 320 x 650 |
| Chat | `chat-{pubkey prefix}` | `/chat?peer={key}` | 480 x 550 |
| Community | `community-{id}` | `/community?id={id}` | 800 x 600 |
| Settings | `settings` | `/settings` | 600 x 500 |
| Profile | `profile-{key prefix}` | `/profile?key={key}` | 400 x 500 |

The buddy list hides to system tray on close rather than exiting. Chat windows
are created dynamically — one per conversation, not tabbed. Closing the login
window while no buddy list is visible triggers application exit.

## Data Flow: Sending a Message

```
┌──────────┐    invoke()     ┌──────────┐   Signal encrypt   ┌──────────────┐
│ Frontend │ ──────────────→ │  Tauri   │ ────────────────→  │rekindle-crypto│
│MessageInput│  send_message │ commands │                    │  (encrypt)   │
└──────────┘                 └────┬─────┘                    └──────┬───────┘
                                  │                                 │
                                  │ ciphertext                      │
                                  ▼                                 │
                            ┌──────────────┐   Cap'n Proto    ┌────┘
                            │rekindle-     │ ←────────────────┘
                            │protocol      │   (serialize)
                            │  (send)      │
                            └──────┬───────┘
                                   │ app_message(route_id, bytes)
                                   ▼
                            ┌──────────────┐
                            │   Veilid     │
                            │  Network     │
                            └──────────────┘
```

## Data Flow: Receiving a Message

```
┌──────────────┐  VeilidUpdate::AppMessage  ┌──────────────┐
│   Veilid     │ ────────────────────────→  │ veilid_      │
│   Network    │                            │ service      │
└──────────────┘                            │ (dispatch)   │
                                            └──────┬───────┘
                                                   │
                                                   ▼
                                            ┌──────────────┐
                                            │ message_     │
                                            │ service      │
                                            │ (process)    │
                                            └──────┬───────┘
                                                   │
                          ┌────────────────────────┤
                          │                        │
                          ▼                        ▼
                   ┌──────────────┐         ┌──────────┐
                   │rekindle-crypto│         │  SQLite  │
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
│  Veilid DHT  │ ────────────────────────→   │ veilid_service │
│  (watched    │                             │ (dispatch)     │
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
