# Frontend Architecture

The SolidJS frontend is a thin UI layer. It renders state received from the
Rust backend and forwards user actions back via Tauri IPC. All business
logic, cryptography, and networking live in Rust.

## Technology

| Component | Technology |
|-----------|-----------|
| Framework | SolidJS (fine-grained reactivity, compiled JSX) |
| Styling | Tailwind CSS 4 (global styles only, no inline classes) |
| Bundler | Vite |
| Language | TypeScript |

## Design Rules

- No inline Tailwind classes — all styling via global CSS with `@apply`
- No inline event handlers — all handlers are named functions in `src/handlers/`
- No business logic in components — state rendering and action forwarding only
- Stores are reactive wrappers around data pushed from Rust via events

## Directory Structure

```
src/
├── main.tsx                          Entry point, path-based routing
├── windows/                          One top-level component per window type (7)
│   ├── LoginWindow.tsx               Passphrase entry, identity creation
│   ├── BuddyListWindow.tsx           Main buddy list (narrow vertical)
│   ├── ChatWindow.tsx                1:1 friend chat (one window per conversation)
│   ├── DmWindow.tsx                  DM / group DM (one window per conversation)
│   ├── CommunityWindow.tsx           Community with channels + members
│   ├── SettingsWindow.tsx            Preferences and configuration
│   └── ProfileWindow.tsx             Friend / member profile viewer
├── components/
│   ├── titlebar/
│   │   └── Titlebar.tsx              Custom frameless window titlebar
│   ├── buddy-list/
│   │   ├── BuddyList.tsx             Friend list container
│   │   ├── BuddyGroup.tsx            Collapsible friend group
│   │   ├── BuddyItem.tsx             Individual friend row
│   │   ├── UserIdentityBar.tsx       Current user identity display
│   │   ├── BottomActionBar.tsx       Action buttons at list bottom
│   │   ├── MenuBar.tsx               Top menu bar
│   │   ├── SearchBar.tsx             Friend search/filter
│   │   ├── TabBar.tsx                Tab navigation (friends, communities, DMs)
│   │   ├── AddFriendModal.tsx        Add friend by public key or invite link
│   │   ├── PublicKeyTab.tsx          Add-friend "by public key" sub-tab
│   │   ├── InviteLinkTab.tsx         Add-friend "by invite link" sub-tab
│   │   ├── DmInviteModal.tsx         Start a new 2-party or group DM
│   │   ├── NewChatModal.tsx          Start a new 1:1 friend chat
│   │   ├── PendingRequests.tsx       Incoming friend request list
│   │   ├── NotificationCenter.tsx    In-app notification display
│   │   └── CommunityListCompact.tsx  Compact community list in buddy sidebar
│   ├── chat/
│   │   ├── MessageList.tsx           Scrollable message history
│   │   ├── MessageBubble.tsx         Individual message display
│   │   ├── MessageRichBody.tsx       Markdown / mentions / link previews renderer
│   │   ├── MessageInput.tsx          Text input with Enter-to-send
│   │   ├── TypingIndicator.tsx       Typing animation
│   │   ├── ReactionBar.tsx           Emoji-reaction strip below messages
│   │   ├── EmojiPicker.tsx           Emoji + custom-emoji picker
│   │   ├── AttachmentDisplay.tsx     File attachment thumbnails / download UI
│   │   ├── PollCard.tsx              Poll voting UI
│   │   ├── ReplyPreview.tsx          "Replying to…" header above input
│   │   ├── ForwardMessageDialog.tsx  Forward to another channel/DM
│   │   ├── ThreadStarter.tsx         Inline "start thread" affordance
│   │   └── VoiceMessagePlayer.tsx    Voice-message playback
│   ├── community/
│   │   ├── CommunityList.tsx         Community sidebar
│   │   ├── ChannelList.tsx           Channel sidebar (per community)
│   │   ├── CategoryHeader.tsx        Channel category collapse header
│   │   ├── MemberList.tsx            Member list with roles
│   │   ├── MemberProfilePopup.tsx    Per-community profile popup
│   │   ├── RoleTag.tsx               Role badge display
│   │   ├── CreateCommunityModal.tsx  Community creation form
│   │   ├── JoinCommunityModal.tsx    Join by invite code
│   │   ├── CreateChannelModal.tsx    Channel creation form
│   │   ├── RenameChannelModal.tsx    Rename channel dialog
│   │   ├── CreateCategoryModal.tsx   Category creation
│   │   ├── RenameCategoryModal.tsx   Category rename
│   │   ├── CreateEventModal.tsx      Scheduled event creation
│   │   ├── EventsPanel.tsx           Upcoming/past events panel
│   │   ├── CreatePollModal.tsx       Poll creation
│   │   ├── ForumChannelView.tsx      Forum-channel thread list view
│   │   ├── ThreadListPanel.tsx       Thread browser
│   │   ├── ThreadPanel.tsx           Single thread message view
│   │   ├── PinnedMessagesPanel.tsx   Pinned messages drawer
│   │   ├── ExpressionPicker.tsx      Emoji/sticker/soundboard picker
│   │   ├── GameServerList.tsx        Community game-server favorites
│   │   ├── StagePanel.tsx            Stage-channel speaker/listener panel
│   │   ├── OnboardingWizard.tsx      First-join onboarding flow
│   │   ├── WelcomeScreen.tsx         Customizable welcome screen
│   │   ├── CommunitySettingsModal.tsx  Settings tab container
│   │   └── settings/
│   │       ├── OverviewTab.tsx
│   │       ├── MembersTab.tsx
│   │       ├── RolesTab.tsx
│   │       ├── PermissionCheckboxList.tsx
│   │       ├── BansTab.tsx
│   │       ├── InvitesTab.tsx
│   │       ├── ChannelsTab.tsx
│   │       ├── AutoModTab.tsx
│   │       ├── AuditLogTab.tsx
│   │       └── SecurityTab.tsx
│   ├── voice/
│   │   ├── VoicePanel.tsx            Voice channel participant panel
│   │   └── VoiceParticipant.tsx      Individual participant display
│   ├── status/
│   │   ├── StatusPicker.tsx          Online/away/busy/invisible dropdown
│   │   ├── StatusDot.tsx             Colored status indicator
│   │   └── NetworkIndicator.tsx      Veilid connection status
│   ├── settings/
│   │   ├── RelaySettingsSection.tsx       Strand Relay configuration
│   │   └── PushRelaySettingsSection.tsx   Mobile push relay configuration
│   └── common/
│       ├── Avatar.tsx                User avatar display
│       ├── ContextMenu.tsx           Right-click context menu
│       ├── ConfirmDialog.tsx         Confirmation dialog
│       ├── Modal.tsx                 Generic modal dialog
│       ├── SimpleInputModal.tsx      Single-input modal (rename, etc.)
│       ├── FormField.tsx             Labeled input with error slot
│       ├── Tooltip.tsx               Hover tooltip
│       ├── Toast.tsx                 Toast notification display
│       └── ScrollArea.tsx            Custom scrollbar container
├── stores/
│   ├── auth.store.ts                 Login state, identity info
│   ├── friends.store.ts              Friend list, presence, groups
│   ├── chat.store.ts                 1:1 conversations, messages, typing
│   ├── dm.store.ts                   DMs / group DMs
│   ├── community.store.ts            Communities, channels, members, threads, events
│   ├── voice.store.ts                Voice connection, mute/deafen, participants
│   ├── notification.store.ts         System notifications
│   ├── settings.store.ts             User preferences
│   ├── relay.store.ts                Strand Relay state (offers, volunteered friends)
│   ├── buddylist-ui.store.ts         Buddy list UI state (search, tabs, modals)
│   ├── toast.store.ts                Toast notification queue
│   └── types.ts                      Shared TS types
├── ipc/
│   ├── commands.ts                   Typed invoke() wrappers (~170 commands)
│   ├── channels.ts                   Event subscriptions via listen()
│   ├── invoke.ts                     Conditional invoke (Tauri native / E2E HTTP)
│   ├── hydrate.ts                    State hydration on login
│   ├── avatar.ts                     Avatar data conversion
│   └── permissions.ts                Permission bitmask constants and helpers
├── handlers/
│   ├── titlebar.handlers.ts          Minimize, maximize, close, hide
│   ├── auth.handlers.ts              Login, create identity, logout
│   ├── buddy.handlers.ts             Double-click, context menu, add friend
│   ├── chat.handlers.ts              Send DM, key handling
│   ├── chat-events.handlers.ts       ChatEvent listener (messages, friend requests, DM invites)
│   ├── dm.handlers.ts                DM-window key + event handlers
│   ├── community.handlers.ts         Create, join, channel actions
│   ├── voice.handlers.ts             Join/leave, mute/deafen
│   ├── settings.handlers.ts          Preference changes
│   ├── relay.handlers.ts             Strand Relay events
│   ├── presence-events.handlers.ts   PresenceEvent listener (online/offline, status, game)
│   ├── notification-events.handlers.ts  NotificationEvent listener
│   └── deep-link.handler.ts          rekindle:// URL handling
├── hooks/
│   └── createContextMenu.ts          Reusable context-menu composable
├── utils/
│   ├── error.ts                      Error formatting
│   ├── formatting.ts                 Text formatting (timestamps, counts)
│   ├── time.ts                       Time formatters
│   ├── color.ts                      Color/hex helpers
│   ├── masking.ts                    Public key masking
│   ├── permissions.ts                Permission bitmask helpers
│   └── transformers.ts               Data shape transformers
├── styles/
│   ├── global.css                    Global Tailwind styles
│   ├── animations.css                Keyframe animations
│   ├── scrollbar.css                 Custom scrollbar styling
│   └── xfire-theme.css               Xfire-inspired theme variables
├── assets/                           Static images / icons
└── icons.ts                          Icon definitions
```

## Routing

Multi-window routing is path-based. Each Tauri window is created with a URL
path. The SolidJS `Switch` in `main.tsx` reads `window.location.pathname` and
renders the matching window component. Window components are lazy-loaded so
each webview only compiles the module tree it renders.

| Path | Window Component |
|------|-----------------|
| `/login` | `LoginWindow` |
| `/buddy-list` | `BuddyListWindow` |
| `/chat?peer={key}` | `ChatWindow` (1:1 friend) |
| `/dm?record={key}` | `DmWindow` (DM / group DM) |
| `/community?id={id}` | `CommunityWindow` |
| `/settings` | `SettingsWindow` |
| `/profile?key={key}` | `ProfileWindow` |

The fallback route renders `LoginWindow`.

## Stores

Stores use SolidJS `createStore()` for reactive state. Each store is
populated by event listeners registered in `channels.ts` and hydrated on
login via `hydrate.ts`.

### auth.store.ts

```
AuthState {
    isLoggedIn: boolean
    publicKey: string | null
    displayName: string | null
    avatarUrl: string | null
    status: 'online' | 'away' | 'busy' | 'offline' | 'invisible'
    statusMessage: string | null
    gameInfo: GameStatus | null
}
```

### friends.store.ts

Friend list, presence, pending requests, and outgoing-invite tracking.

### chat.store.ts

1:1 friend conversations keyed by peer public key. Messages, typing state,
last-read timestamps.

### dm.store.ts

DMs and group DMs keyed by SMPL record key. Holds pending invites awaiting
accept/decline.

### community.store.ts

Joined communities, channel lists, member lists, role definitions, threads,
events, pins, expressions, and per-channel unread counts.

### voice.store.ts

Voice connection state: channel ID, mute/deafen, participant list,
connection quality, device selection, active call type (`dm` / `community`).

### relay.store.ts

Strand Relay state: received offers (friends volunteering to relay for us)
and volunteered offers (friends we relay for).

### notification.store.ts / settings.store.ts / toast.store.ts / buddylist-ui.store.ts

Notifications inbox, user preferences, transient toast queue, and buddy-list
UI state (search query, active tab, open modals).

## IPC Layer

### commands.ts

Typed wrappers around `invoke()` for all Tauri commands. Each function maps
directly to a `#[tauri::command]` in the Rust backend.

### channels.ts

Event subscriptions using `listen()` from `@tauri-apps/api/event`:

| Event Name | Enum Type | Updates |
|------------|-----------|---------|
| `chat-event` | `ChatEvent` | Messages, typing, friend requests, DM invites |
| `presence-event` | `PresenceEvent` | Online/offline, status, game changes |
| `voice-event` | `VoiceEvent` | Join/leave, speaking, mute, device change |
| `notification-event` | `NotificationEvent` | System alerts, update notifications |
| `community-event` | `CommunityEvent` | Member changes, MEK rotation, kicks, role changes, threads, events, video, soundboard, raids, … |
| `network-status` | `NetworkStatusEvent` | Veilid attachment state, DHT readiness, route status |
| `profile-updated` | (no payload) | Triggers frontend to re-fetch profile data |

In E2E testing mode (`VITE_E2E=true`), `safeListen()` is a no-op because the
Tauri event system is not available in a browser context.

### invoke.ts

Conditional invoke wrapper. In production, delegates to
`@tauri-apps/api/core` invoke. In E2E mode (`VITE_E2E=true`), sends HTTP
POST to the E2E bridge server at `http://127.0.0.1:3001/invoke` (provided by
the `rekindle-e2e-server` crate). Window-navigation commands trigger
browser `location.href` changes in E2E mode.

## Handler Pattern

All event handlers are named, module-level functions in `src/handlers/`.
Components reference handlers by name — no inline arrow functions. This
enforces separation between rendering and action forwarding.

```
Component                    Handler                     IPC
─────────────────────────────────────────────────────────────────
<MessageInput />  ──→  chat.handlers.ts       ──→  commands.ts
                       handleSendMessage()         sendMessage()
                       handleKeyDown()
```

## Hooks and Utilities

`hooks/createContextMenu.ts` provides a reusable composable for context-menu
state and outside-click handling. `utils/` contains formatting helpers,
time/color/mask helpers, transformers between IPC payload shapes and store
shapes, and permission-bitmask helpers shared with `ipc/permissions.ts`.
