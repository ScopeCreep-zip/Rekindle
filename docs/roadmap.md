# Implementation Roadmap

This roadmap is split into three tracks: the original 1:1 messaging
phases (substantially complete), the **Communities v2.0** flat-SMPL
governance work, and the **Crate harvest** that has produced eleven
new tier-aligned crates since May 2026.

Status legend: `[x]` done · `[~]` in progress · `[ ]` not started.

## Phase 1: Foundation

**Goal:** Tauri scaffolding, Veilid node startup, identity creation,
frameless window with custom titlebar.

- [x] Tauri 2 project scaffold with SolidJS
- [x] Konductor Nix flake integration
- [x] Frameless transparent window with custom titlebar
- [x] Veilid node startup and attach
- [x] Ed25519 identity generation
- [x] Vault (`rekindle-vault`, SQLCipher) creation and unlock — per-identity files; replaces the original `iota_stronghold` path
- [x] SQLite database initialisation
- [x] Login / logout flow
- [x] Multi-identity support (list, select, delete)
- [x] System tray with status menu
- [x] DHT profile record creation and publishing

## Phase 2: Friends and Chat

**Goal:** Add friends by public key, Signal Protocol session
establishment, end-to-end encrypted 1:1 messaging, separate chat
windows.

- [x] Friend request send / receive / accept / reject via Veilid
- [x] PreKeyBundle generation and DHT publishing
- [ ] PreKey rotation and one-time prekey replenishment
- [x] Signal Protocol session establishment (X3DH)
- [x] Message encrypt → envelope → Veilid send
- [x] Message receive → deserialise → decrypt → SQLite store
- [x] Chat window (MessageList, MessageBubble, MessageInput)
- [x] Multi-window chat (one window per conversation)
- [x] Typing indicators (ephemeral, not queued)
- [x] Presence watching via DHT (online / offline status dots)
- [x] System notifications on new messages
- [x] Message history persistence in SQLite
- [x] Offline message queue (`pending_messages` with retry)
- [x] Friend groups (create, rename, move friends)
- [x] Conversation DHT records (per-friend pair)
- [x] Block / unblock / cancel-request / outgoing-invite tracking
- [x] Invisible status

## Phase 3: Game Detection

**Goal:** Detect running games, display game info on buddy list,
publish to DHT, integrate with community game-server favourites.

- [x] Platform process scanning (sysinfo)
- [x] JSON game database (process name → game info)
- [x] Configurable scan interval
- [x] DHT profile subkey 4 publish on game change
- [x] Buddy list UI ("Playing: Game Name")
- [x] Server-address tracking (`launch_game_to_server`)
- [x] Community game-server favourites (`game_servers` table + UI)
- [ ] Game time tracking (elapsed, persisted to SQLite)
- [ ] Rich presence (server info display)

## Phase 4: Communities v1.0 → v2.0 Migration

The original v1.0 coordinator child-process architecture has been
**removed**. Communities now use the **Communities v2.0**
flat-governance model:

- No coordinator, no privileged nodes — every member is a full peer
- All shared state lives in SMPL DHT records with `o_cnt: 0`
- Three-path delivery: SMPL write (durable) + gossip mesh (fast) +
  watch / inspect (consistent)
- CRDT merge of `GovernanceEntry` variants (`rekindle-governance`)
  with the async lifecycle layered on top in
  `rekindle-governance-runtime`
- Reader-validates permissions
- Per-channel MEKs distributed via the SMPL member-registry MEK
  vault
- Deterministic MEK rotation rotator
  (`rekindle-mek-rotation`, blake3 lowest-hash wins)
- Plate Gates for >255 members (fractal SMPL segments)

### Sub-phase status

- [x] **Phase 1 — Foundation crates:** `rekindle-types`,
  `rekindle-secrets`, `rekindle-governance`, `rekindle-codec`,
  `rekindle-records` extracted; workspace lints pass clean.
- [x] **Phase 2 — Flat governance:** Coordinator process removed,
  CRDT merge wired, reader-validates permissions, self-sovereign
  join, governance commands ported, BootstrapBundle handler.
- [x] **Phase 3 — Three-path delivery:** SMPL write + gossip mesh +
  inspect-polling all wired (`rekindle-gossip`, `rekindle-sync`,
  `rekindle-route`). Hardening (rate-limiting integration, history
  advertisement, route-context selection) is largely in place.
- [x] **Phase 4 — Peer MEK distribution:** Per-channel MEK cache
  (`channel_mek_cache`), deterministic rotator (`rekindle-secrets::rotator`),
  MEK rotation pipeline (`rekindle-mek-rotation` + Tauri-side
  `services/community/mek_rotation.rs`), vault persistence,
  cascade fallback.
- [x] **Phase 5 — Rich features:** Threads, polls, reactions, pins,
  attachments (Lost Cargo `rekindle-files`), custom emoji /
  stickers / soundboard, link previews
  (`rekindle-link-preview`), AutoMod, audit log, scheduled events
  with RSVPs and reminders, raid detection, per-community
  profiles (bio / pronouns / theme colour / badges / avatar /
  banner), forum channels, stage channels, video / screen-share
  (`rekindle-video`), DMs and group DMs (`rekindle-dm`).

### Open community work

- [ ] Cross-device sync productionisation (subkey reconciliation
  tests, conflict UX)
- [ ] Push relay end-to-end testing on mobile target platforms
- [ ] Plate Gate segment expansion stress-testing for >1000 members
- [ ] **C1-2** lazy per-segment channel records +
  `ChannelSegmentLinked` governance entry. Until this lands,
  offline catch-up for members in segments ≥1 falls back to
  gossip-only delivery — see [`architecture/communities-governance.md`](architecture/communities-governance.md).
- [ ] Community browser / discovery (no public directory yet)
- [ ] Updater wiring (`check_for_updates` is currently a stub)

### Key documents

| Document | Purpose |
|----------|---------|
| [`architecture/communities.md`](architecture/communities.md) | Three-part Communities spec index |
| [`architecture/communities-overview.md`](architecture/communities-overview.md) | Model, three-path delivery, SMPL schema, permissions, design principles |
| [`architecture/communities-channels.md`](architecture/communities-channels.md) | MEK, channel messaging, voice, DMs |
| [`architecture/communities-governance.md`](architecture/communities-governance.md) | CRDT governance, join, Plate Gate scaling |
| [`architecture/overview.md`](architecture/overview.md) | System layer stack and data flows |
| [`protocol/overview.md`](protocol/overview.md) | Wire formats, Veilid primitives, DHT layouts |
| [`security/overview.md`](security/overview.md) | Five-layer encryption stack and threat model |

## Phase 5: Voice

**Goal:** Voice channels in communities, 1:1 voice calls, Opus
codec with acceptable latency.

- [x] Audio capture via cpal (dedicated thread)
- [x] Audio playback via cpal (dedicated thread)
- [x] Opus encode / decode (48 kHz mono, VoIP mode, 32 kbps, FEC)
- [x] Voice activity detection (energy-based + RNNoise denoising)
- [x] Jitter buffer (adaptive, BTreeMap-by-sequence)
- [x] Audio mixer (multi-participant)
- [x] Voice transport over Veilid (`SafetySelection::Unsafe`)
- [x] Join / leave voice channel commands
- [x] Mute / deafen controls
- [x] Global shortcut: `Ctrl+Shift+M` toggle mute
- [x] Voice panel UI (participants, speaking indicators)
- [x] 1:1 voice calls from chat window
- [x] Audio processing pipeline (RNNoise + AEC3 echo cancellation)
- [x] Audio device selection (input / output)
- [x] Stage channels (listener / speaker / hand-raise)
- [x] Server-side mute / deafen (`server_mute_member`,
  `server_deafen_member`)
- [x] Voice mode switching (`set_voice_mode`)
- [x] Voice session join / leave analytics
  (`voice_session_events`)
- [x] Call signaling reliability layer (W13 fire-and-forget +
  W16 `pending_envelopes` retry queue, per-recipient seq_ack,
  receiver dedup, crash-recovers Dialing / Incoming)
- [x] Backend-owned call state machine and authoritative event
  emit (W14 / W15) — every frontend renders identical lifecycle
  from the same event stream
- [x] AEAD audio encryption under derived `call_key` (W13.14)
- [x] Phase 14.q `CallRegistry` trait + `active_calls` adapter
  surface in `AppState`
- [ ] Connection quality monitoring and display

## Phase 6: Advanced Features

**Goal:** File sharing, deep links, autostart, push relay, screen
share, overlay, auto-update.

- [x] Autostart (`tauri-plugin-autostart`, LaunchAgent on macOS)
- [x] Deep link registration and invite handling
  (`rekindle://invite/{blob}`)
- [x] Ed25519-signed invite blobs (generate, verify, base64url
  encode)
- [x] Block list
- [x] Mailbox DHT records (route blob fallback for offline peers)
- [x] File sharing via Veilid (Lost Cargo — `rekindle-files`)
- [x] Strand Relay forwarding (architecture §13)
- [x] Mobile push relay client (`push_relay`)
- [x] Cross-device sync foundation (architecture §28.4)
- [x] Video / screen-share fragmentation pipeline
  (`rekindle-video`)
- [x] Full-text search (FTS5 across messages, threads, DMs)
- [ ] Auto-update via Tauri updater (`check_for_updates` stubbed)
- [ ] In-game overlay (research / prototype)

## Harvest crates (Phase 23 services drain)

The harvest is an ongoing decomposition of `src-tauri/src/services/`
into tier-aligned crates with the runtime / adapter / pure-logic
split documented in
[`architecture/services-pattern.md`](architecture/services-pattern.md).
Eleven crates have shipped; the services-drain frontier is the
live work.

- [x] `rekindle-events` — `EventDedup` + `SubscriptionState` +
  `EventJournal` hoisted out of `rekindle-transport`
- [x] `rekindle-vault` — SQLCipher double-encrypted store replacing
  `iota_stronghold`; see
  [`decisions/0006-vault-replaces-stronghold.md`](decisions/0006-vault-replaces-stronghold.md)
- [x] `rekindle-audit` — BLAKE3 keyed hash chain for tamper-evident
  audit log; see [`architecture/audit-chain.md`](architecture/audit-chain.md)
- [x] `rekindle-lifecycle` — 9-state app FSM + `TransportGuard`
  hoisted from the daemon; see
  [`architecture/lifecycle-fsm.md`](architecture/lifecycle-fsm.md)
- [x] `rekindle-friendship` — three-tier inbox-scan coordinator
- [x] `rekindle-idempotency` — LRU + TTL command-dedup cache
- [x] `rekindle-mek-rotation` — deterministic blake3 election +
  cascade distribute
- [x] `rekindle-governance-runtime` — async lifecycle on top of
  the pure-CRDT `rekindle-governance`
- [x] `rekindle-channel` — channel messaging + threads + reactions
  + polls + automod + slowmode
- [x] `rekindle-presence` — friend + community + idle + game
  presence orchestrators; see
  [`architecture/presence.md`](architecture/presence.md)
- [x] `rekindle-analytics` — local-only SQL aggregations
- [~] **Phase 23 services drain** — ongoing harvest of remaining
  `src-tauri/src/services/community/` logic into the appropriate
  Tier-7 crates; ~37.6 K LoC of services with the runtime /
  adapter / pure-logic split applied per surface
- [x] **Phase 23.A** event_dispatch single-source emit router;
  see [`architecture/event-dispatch.md`](architecture/event-dispatch.md)
- [x] **Phase 23.B** `state.rs` split into `state/` submodules;
  AppState now ~47 fields in `state/app_state.rs`
- [~] **Phase 23.C** `*_runtime.rs` orchestration extraction —
  channel, community lifecycle, MEK local-rotate, others ongoing
- [~] **Phase 23.D** adapter module-dir pattern (Phase 14.r / 23.D)
  applied to `channel_adapter/`, `calls_adapter/`, others ongoing

## Known Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Veilid DHT latency (500 ms–5 s) | Slow presence updates | Aggressive SQLite caching + `watch_dht_values` + inspect polling fallback |
| Voice latency over privacy routes | Unusable voice | `SafetySelection::Unsafe` for voice (direct UDP-like) |
| Veilid API maturity | Breaking changes | Isolate Veilid behind handles in `rekindle-protocol` / `rekindle-transport` |
| MEK distribution at scale | Slow rotation cascades | Deterministic rotator + per-channel MEK + Plate Gates |
| Cross-platform audio | cpal issues on Linux, macOS permissions | Dedicated threads, `mpsc` bridge, hot-swap detection |
| DHT value size limits | Large community records | SMPL multi-subkey layout + DHTLog spine for channel history |
| FTS storage growth | Disk usage on busy users | External-content FTS5 with triggers; periodic vacuum |
| Push relay battery / metadata | Excessive wake-up cost | 30 s wake-notify debounce, content-free wake signal |
