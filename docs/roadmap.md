# Implementation Roadmap

Development is organized into six phases. Each phase builds on the previous and
includes verification criteria.

## Phase 1: Foundation

**Goal:** Tauri scaffolding, Veilid node startup, identity creation, frameless
window with custom titlebar.

- [x] Tauri 2 project scaffold with SolidJS
- [x] Konductor Nix flake integration
- [x] Frameless transparent window with custom titlebar
- [x] Veilid node startup and attach
- [x] Ed25519 identity generation
- [x] Stronghold vault creation and unlock
- [x] SQLite database initialization
- [x] Login/logout flow
- [x] Multi-identity support (list, select, delete)
- [x] System tray with status menu
- [x] DHT profile record creation and publishing

**Verification:** Application starts, creates identity, logs in, shows buddy
list, Veilid node attaches to network.

## Phase 2: Friends and Chat

**Goal:** Add friends by public key, Signal Protocol session establishment,
end-to-end encrypted 1:1 messaging, separate chat windows.

- [x] Friend request send/receive/accept/reject via Veilid
- [x] PreKeyBundle generation and DHT publishing
- [ ] PreKey rotation and one-time prekey replenishment
- [x] Signal Protocol session establishment (X3DH)
- [x] Message encrypt → Cap'n Proto serialize → Veilid send
- [x] Message receive → deserialize → decrypt → SQLite store
- [x] Chat window (MessageList, MessageBubble, MessageInput)
- [x] Multi-window chat (one window per conversation)
- [x] Typing indicators (ephemeral, not queued)
- [x] Presence watching via DHT (online/offline status dots)
- [x] System notifications on new messages
- [x] Message history persistence in SQLite
- [x] Offline message queue (pending_messages with retry)
- [x] Friend groups (create, rename, move friends)
- [x] Conversation DHT records (per-friend pair)

**Verification:** Two instances exchange end-to-end encrypted messages. Messages
persist across restarts. Separate chat windows open per friend. Friend comes
online — status dot updates.

## Phase 3: Game Detection

**Goal:** Detect running games, display game info on buddy list, publish to DHT.

- [x] Platform process scanning (sysinfo)
- [x] JSON game database (process name → game info)
- [x] Configurable scan interval
- [x] DHT profile subkey 4 publish on game change
- [x] Buddy list UI ("Playing: Game Name")
- [ ] Game time tracking (elapsed, stored in SQLite)
- [ ] Rich presence (server info display)

**Verification:** Launch a known game — buddy list shows game info. Friend sees
"Playing X" on their buddy list.

## Phase 4: Communities

**Goal:** Create and join communities with text channels, group encryption,
roles, and permissions.

- [x] Community creation (DHT record, metadata, server process)
- [x] Join by invite code via community server RPC
- [x] Text channel management (create/delete/rename via server RPC)
- [x] Channel messaging via community server (server relays to members)
- [x] Role system with permission bitmasks (create/edit/delete/assign/unassign)
- [x] Community window UI (channel sidebar, message area, member list, settings)
- [x] Member management via server (kick, ban, unban, timeout)
- [x] Community server process (`rekindle-server` child daemon)
- [x] Server health monitoring (30s ping, auto-restart on failure)
- [x] Community broadcasts (NewMessage, MemberJoined/Removed, RolesChanged, etc.)
- [x] Per-channel permission overwrites (role/member allow/deny bitmasks)
- [x] Community pseudonyms (unlinkable identity per community via HKDF)
- [~] MEK rotation via server RPC (command exists, distribution pipeline partial)
- [ ] MEK storage in Stronghold (generated but not persisted)
- [ ] MEK distribution to members via Signal sessions
- [ ] Full MEK-encrypted channel messaging (primitives exist, pipeline not wired)
- [ ] Community browser (discover public communities)
- [x] Community invites via deep link (`rekindle://invite/{blob}`)

**Verification:** Create community, invite friend, exchange channel
messages via server relay. Roles and bans work. MEK encryption not yet active.

**Current status note:** The community system now uses a client-server
architecture within the P2P network. The community owner spawns a
`rekindle-server` child process that handles RPC (join, messaging, moderation)
and broadcasts events to all members. Channel messages route through the server
process. MEK encrypt/decrypt primitives exist in `rekindle-crypto` but the
end-to-end pipeline (Stronghold storage → Signal-session distribution →
per-message encryption) is not yet wired.

## Phase 5: Voice

**Goal:** Voice channels in communities, 1:1 voice calls, Opus codec with
acceptable latency.

- [x] Audio capture via cpal (dedicated thread)
- [x] Audio playback via cpal (dedicated thread)
- [x] Opus encode/decode (48kHz mono)
- [x] Voice activity detection (energy-based + RNNoise denoising)
- [x] Jitter buffer (adaptive)
- [x] Audio mixer (multi-participant)
- [x] Voice transport over Veilid (unsafe safety selection)
- [x] Join/leave voice channel commands
- [x] Mute/deafen controls
- [x] Global shortcut: Ctrl+Shift+M toggle mute
- [x] Voice panel UI (participants, speaking indicators)
- [x] 1:1 voice calls from chat window
- [x] Audio processing pipeline (RNNoise denoising + AEC3 echo cancellation)
- [x] Audio device selection (input/output)
- [ ] Connection quality monitoring and display

**Verification:** Join voice channel — audio flows between participants.
One-way latency < 200ms. VAD correctly detects speech vs silence. Speaking
indicator shows who is talking.

## Phase 6: Advanced Features

**Goal:** File sharing, deep links, auto-update, autostart, overlay research.

- [x] Autostart (tauri-plugin-autostart, LaunchAgent on macOS)
- [x] Deep link registration and invite handling (`rekindle://invite/{blob}`)
- [x] Ed25519-signed invite blobs (generate, verify, base64url encode/decode)
- [x] Block list (drop messages from blocked users)
- [x] Mailbox DHT records (route blob fallback for offline peers)
- [ ] File sharing via Veilid P2P
- [ ] Auto-update via Tauri updater
- [ ] Screen share (research/prototype)
- [ ] In-game overlay (research/prototype)

## Known Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Veilid DHT latency (500ms-5s) | Slow presence updates | Aggressive SQLite caching + `watch_dht_values` |
| Voice latency over privacy routes | Unusable voice | `SafetySelection::Unsafe` for voice (direct UDP) |
| Veilid API maturity | Breaking changes | Isolate Veilid behind trait in `rekindle-protocol` |
| Group encryption at scale | Slow MEK distribution | Cap ~100 members; TreeKEM planned for larger |
| Cross-platform audio | cpal issues on Linux, macOS permissions | Test early; platform-specific workarounds |
| DHT value size limits | Large community records | Record chaining / pagination |
