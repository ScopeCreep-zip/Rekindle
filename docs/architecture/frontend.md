# Frontend Architecture

The SolidJS frontend is a thin UI layer. It renders state received
from the Rust backend and forwards user actions back via Tauri IPC.
All business logic, cryptography, and networking live in Rust.

## Technology

| Component | Technology |
|-----------|-----------|
| Framework | SolidJS (fine-grained reactivity, compiled JSX) |
| Styling | Tailwind CSS 4 (global styles only, no inline classes) |
| Bundler | Vite |
| Language | TypeScript |

## Design Rules

- **No inline Tailwind classes.** All styling lives in
  `src/styles/global.css` and is applied via `@apply` or class
  selectors. The CI lint catches inline class violations.
- **No inline event handlers.** Components reference named handlers
  from `src/handlers/` so handler bodies stay searchable and
  testable.
- **No business logic in components.** Components render state and
  forward actions only. Stores are reactive wrappers around state
  pushed from Rust via events.
- **No silent component lazy-loading inside windows.** All eight
  windows are lazy-loaded at the routing boundary in
  `src/main.tsx`; everything inside a window is eager.

## Routing and Window Mounts

`src/main.tsx` reads `window.location.pathname` and renders the
matching window component. Each Tauri window has its own URL path
and its own lazy chunk.

| Path prefix | Window component |
|---|---|
| `/login` | `LoginWindow` |
| `/buddy-list` | `BuddyListWindow` |
| `/chat` | `ChatWindow` |
| `/dm` | `DmWindow` |
| `/community` | `CommunityWindow` |
| `/settings` | `SettingsWindow` |
| `/profile` | `ProfileWindow` |
| `/call` | `CallWindow` |

`main.tsx` also mounts `CallController` and `AnnounceRegion` globally
so incoming call notifications surface across every window and
screen-reader announcements work on every page.

## Directory Structure

```
src/
├── main.tsx                          Entry point, path-based routing,
│                                     global CallController + AnnounceRegion
├── windows/                          One top-level component per window (8)
│   ├── LoginWindow.tsx
│   ├── BuddyListWindow.tsx
│   ├── ChatWindow.tsx
│   ├── DmWindow.tsx
│   ├── CommunityWindow.tsx           Composes community_window/ panes
│   ├── SettingsWindow.tsx            Composes settings/ tabs
│   ├── ProfileWindow.tsx
│   ├── CallWindow.tsx
│   ├── community_window/             Pane decomposition for CommunityWindow
│   │   ├── CommunityMainPane.tsx
│   │   ├── CommunityRightPanel.tsx
│   │   ├── CommunitySidebar.tsx
│   │   ├── CommunityModals.tsx
│   │   ├── state.ts                  CommunityWindow-local store
│   │   └── useCommunityWindow.ts     Composable wiring hook
│   └── settings/                     Tabs for SettingsWindow
│       ├── ApplicationTab.tsx
│       ├── AudioTab.tsx              Microphone, output, AEC, RNNoise toggles
│       ├── VideoTab.tsx
│       ├── NotificationsTab.tsx
│       ├── PrivacyTab.tsx
│       ├── DevicesTab.tsx            Cross-device sync devices
│       ├── ProfileTab.tsx
│       └── AboutTab.tsx
├── components/                       Reusable UI components by feature
│   ├── titlebar/
│   │   └── Titlebar.tsx              Custom frameless titlebar
│   ├── buddy-list/                   Friend list, groups, DM tab, modals
│   ├── chat/                         Message list, bubbles, input, polls,
│   │                                 reactions, attachments, search
│   ├── community/                    Channels, members, settings tabs,
│   │                                 events, threads, stage, onboarding
│   ├── voice/                        Call surfaces — 1:1, group, video,
│   │                                 stage, soundboard, reactions
│   ├── status/                       Status picker, dot, network indicator
│   ├── settings/                     Relay + push relay + QR scanner
│   └── common/                       Avatar, modal, scroll area, toast,
│                                     announce region, live region, tooltip
├── stores/                           SolidJS reactive state (16 stores)
├── handlers/                         Named event-handler functions
├── hooks/                            Reusable composables
├── ipc/                              Tauri command + event bridges
├── icons.ts                          Icon definitions
├── styles/                           Global CSS (Tailwind @apply)
└── assets/                           Static assets
```

## Components by Feature

### `buddy-list/`

Friend list container, identity bar, add-friend modal (with
public-key and invite-link sub-tabs), pending requests, in-app
notification centre, compact community list, search and tab bars.
The `StartGroupCallModal` initiates the group-call flow that powers
the call surfaces in `voice/`.

### `chat/`

Message list and bubble, rich body renderer (markdown, mentions,
link previews), message input (with a `message_input/` subdirectory
for the per-section internals), reactions, attachment display, poll
card, reply preview, thread starter, forward dialog, voice message
player, typing indicator. `ChannelChat.tsx` is the community
channel surface; `SearchPanel.tsx` is the FTS5 search UI for both
DMs and channels.

### `community/`

Channel list and category headers, member list with member-profile
popup, the main create-community / join-community modals, channel /
category / role / thread / event / poll create-and-rename modals,
expression picker, forum view, pinned messages panel, stage panel,
thread list + thread panel, onboarding wizard, welcome screen,
join progress stepper, game-server list, role tag.

The `community/settings/` subdirectory carries the settings modal's
tabs: Overview, Members, Roles, Bans, Invites, Channels, AutoMod,
AuditLog, Analytics, Security. The `permissions checkbox list` is
shared across the role and channel overwrite editors. The
`channels_tab/` subdirectory holds the channel-tab internals
(channel-row, permission editor, overwrite editor).

### `voice/`

Sixteen files cover every active-call surface. `CallController` is
the global lifecycle controller mounted by `main.tsx`. The
panels — `VoicePanel`, `ActiveCallPanel`, `GroupCallPanel`,
`VideoCallPanel`, `IncomingCallModal`, `OutgoingCallPanel`,
`CallWaitingBanner` — render call state. Auxiliary surfaces:
`VoiceParticipant`, `ReactionsTray`, `ReactionFloater`,
`SoundboardPanel`. The `call_stage/` subdirectory holds the stage-
channel internals, and `video_call/` holds the video-tile grid.

### `status/`

`StatusPicker`, `StatusDot`, `NetworkIndicator`. The picker writes
through `commands.setStatus`; the indicator reflects the
`network-status` event payload.

### `settings/` (top-level, not the community settings tabs)

`RelaySettingsSection`, `PushRelaySettingsSection`,
`AddDeviceModal`, `QrScannerOverlay`. These live at the top level
so the buddy list and settings window can both surface relay
controls.

### `common/`

Shared primitives: `Avatar`, `Modal`, `SimpleInputModal`,
`ScrollArea`, `Toast`, `Tooltip`, `LoadingButton`, `ConfirmDialog`,
`FormField`, `AnnounceRegion`, `LiveRegion`.

### `titlebar/`

`Titlebar` — the custom frameless window titlebar applied to every
window because `decorations: false` is the project's frameless
Xfire skin baseline.

## Stores

Sixteen SolidJS reactive stores live in `src/stores/`:

| Store | Purpose |
|---|---|
| `auth.store.ts` | Identity, session, login state |
| `chat.store.ts` | 1:1 chat conversations |
| `dm.store.ts` | DM / group DM conversations |
| `friends.store.ts` | Friend list, groups, presence |
| `community.store.ts` | Communities, channels, members, governance |
| `voice.store.ts` | Voice engine state (mute, deafen, devices, channel) |
| `calls.store.ts` | Active call registry mirror |
| `notification.store.ts` | Notification preferences, in-app history |
| `relay.store.ts` | Strand Relay state |
| `settings.store.ts` | User preferences |
| `link_preview.store.ts` | OpenGraph preview cache |
| `buddylist-ui.store.ts` | Buddy list UI state (tab, search) |
| `join.store.ts` | Community join flow progress |
| `lifecycle.store.ts` | App lifecycle mirror (mirrors `rekindle-lifecycle`) |
| `toast.store.ts` | Toast queue |
| `types.ts` | Shared store types |

The stores are kept thin — they hold mirrored state, not derived
state. Derivations happen in components via SolidJS reactive
primitives (`createMemo`, `createComputed`).

## Handlers

Thirty-plus handler files in `src/handlers/` cover the IPC
dispatch surface. Files at the top level group handlers by domain:

- `auth.handlers.ts`, `chat.handlers.ts`, `chat-events.handlers.ts`,
  `dm.handlers.ts`, `voice.handlers.ts`, `buddy.handlers.ts`,
  `settings.handlers.ts`, `relay.handlers.ts`, `titlebar.handlers.ts`,
  `notification-events.handlers.ts`, `presence-events.handlers.ts`,
  `deep-link.handler.ts`.
- `calls.handlers.ts` plus the `calls/` subdirectory (`actions.ts`,
  `events.ts`, `ring.ts`) cover the Phase 14.q call lifecycle.
- `community.handlers.ts` plus the `community/` subdirectory cover
  the community surface in depth: `channels.ts`, `dispatcher.ts`
  and the per-concern dispatchers (`dispatcher_content.ts`,
  `dispatcher_members.ts`, `dispatcher_messages.ts`,
  `dispatcher_voice.ts`), `events_threads.ts`, `invites.ts`,
  `lifecycle.ts`, `messages.ts`, `moderation.ts`, `profile.ts`,
  `roles.ts`, `shared.ts`.

## IPC Layer

`src/ipc/commands.ts` and `src/ipc/channels.ts` are the master
entry points. Both delegate to per-domain modules so adding a new
command or event class does not bloat a single file.

### `ipc/commands/`

| File | Domain |
|---|---|
| `account.ts` | Identity creation, login, logout, list / delete |
| `community.ts` | Community CRUD, channels, members, roles, moderation, threads, events, polls, reactions, expressions, files |
| `governance.ts` | Governance state queries and admin commands |
| `sync.ts` | Cross-device sync and pairing |
| `system.ts` | Settings, push relay, system status, deep links |
| `voice.ts` | Voice and call commands |
| `types.ts` + `types_sync.ts` | Shared command + sync types |

### `ipc/channels/`

| File | Veilid-side channel |
|---|---|
| `chat_events.ts` | `chat-event` — 1:1 chat, friend requests, calls |
| `community_events.ts` | `community-event` — 50+ variants |
| `community_video_events.ts` | High-throughput video frames (separate from `community-event` to avoid hot-path overhead) |
| `notification_events.ts` | `notification-event` + `network-status` |
| `presence_events.ts` | `presence-event` |
| `voice_events.ts` | `voice-event` |
| `subscriptions.ts` | The `safeListen` wrapper used by every channel |

`safeListen` is a no-op in E2E mode (`VITE_E2E=true`) because
there is no Tauri event system in the browser-driven Playwright
runs.

### IPC adapters at the root

| File | Purpose |
|---|---|
| `invoke.ts` | Conditional invoke (Tauri normally, HTTP to `localhost:3001` under E2E) |
| `hydrate.ts` | State hydration on login |
| `avatar.ts` | Avatar data handling (Tauri convertFileSrc adapter) |
| `permissions.ts` | Permission bitmask helpers shared with the backend bitfield |

## Styles

Global CSS lives in `src/styles/` and is the **only** place
styling rules can be authored. The current files:

- `global.css` — Tailwind base + project tokens.
- `xfire-theme.css` — Xfire colour palette and skin (see
  [`ui-skin.md`](ui-skin.md)).
- `animations.css` — keyframe animations.
- `scrollbar.css` — scrollbar overrides for the Xfire skin.

Components reference theme classes by name. The lint enforces
that no JSX `className` literal contains a Tailwind utility class.

## Event-driven Updates

Every Rust → Frontend event flows through the
[event-dispatch router](event-dispatch.md). On Tauri side every
emit is routed through `event_dispatch::EventDispatch`; on the
frontend side `subscriptions.ts::safeListen` registers the
listener. The frontend persists the latest cursor to
`localStorage` via the dedicated `cursor-tick` channel so soft
stalls (page reload, IPC pause) replay missed events through the
`event_resume` Tauri command without a full re-hydration.
