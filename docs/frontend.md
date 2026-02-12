# Frontend Architecture

The SolidJS frontend is a thin UI layer. It renders state received from the Rust
backend and forwards user actions back via Tauri IPC. All business logic,
cryptography, and networking live in Rust.

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
├── windows/                          One top-level component per window type
│   ├── LoginWindow.tsx               Passphrase entry, identity creation
│   ├── BuddyListWindow.tsx           Main buddy list (narrow vertical)
│   ├── ChatWindow.tsx                1:1 chat (one per conversation)
│   ├── CommunityWindow.tsx           Community with channels + members
│   ├── SettingsWindow.tsx            Preferences and configuration
│   └── ProfileWindow.tsx             Friend profile viewer
├── components/
│   ├── titlebar/
│   │   └── Titlebar.tsx              Custom frameless window titlebar
│   ├── buddy-list/
│   │   ├── BuddyList.tsx             Friend list container
│   │   ├── BuddyGroup.tsx            Collapsible friend group header
│   │   ├── BuddyItem.tsx             Individual friend row
│   │   ├── UserIdentityBar.tsx       Current user's identity display
│   │   ├── BottomActionBar.tsx       Action buttons at list bottom
│   │   ├── AddFriendModal.tsx        Add friend by public key
│   │   ├── PendingRequests.tsx       Incoming friend request list
│   │   ├── NewChatModal.tsx          Start new conversation
│   │   └── NotificationCenter.tsx    In-app notification display
│   ├── chat/
│   │   ├── MessageList.tsx           Scrollable message history
│   │   ├── MessageBubble.tsx         Individual message display
│   │   ├── MessageInput.tsx          Text input with Enter-to-send
│   │   └── TypingIndicator.tsx       Typing animation
│   ├── community/
│   │   ├── CommunityList.tsx         Community browser
│   │   ├── ChannelList.tsx           Channel sidebar
│   │   ├── MemberList.tsx            Member list with roles
│   │   ├── RoleTag.tsx               Role badge display
│   │   ├── CreateCommunityModal.tsx  Community creation form
│   │   ├── CreateChannelModal.tsx    Channel creation form
│   │   └── JoinCommunityModal.tsx    Join by invite code
│   ├── voice/
│   │   ├── VoicePanel.tsx            Voice channel participant panel
│   │   └── VoiceParticipant.tsx      Individual participant display
│   ├── status/
│   │   ├── StatusPicker.tsx          Online/away/busy dropdown
│   │   ├── StatusDot.tsx             Colored status indicator
│   │   └── NetworkIndicator.tsx      Veilid connection status
│   └── common/
│       ├── Avatar.tsx                User avatar display
│       ├── ContextMenu.tsx           Right-click context menu
│       ├── Modal.tsx                 Generic modal dialog
│       ├── Tooltip.tsx               Hover tooltip
│       └── ScrollArea.tsx            Custom scrollbar container
├── stores/
│   ├── auth.store.ts                 Login state, identity info
│   ├── friends.store.ts              Friend list, presence, groups
│   ├── chat.store.ts                 Conversations, messages, typing
│   ├── community.store.ts            Communities, channels, members
│   ├── voice.store.ts                Voice connection, mute/deafen
│   ├── settings.store.ts             User preferences
│   └── notification.store.ts         System notifications
├── ipc/
│   ├── commands.ts                   Typed invoke() wrappers for all commands
│   ├── channels.ts                   Event subscriptions via listen()
│   ├── invoke.ts                     Conditional invoke (Tauri native / E2E HTTP)
│   ├── hydrate.ts                    State hydration on login
│   └── avatar.ts                     Avatar data conversion
├── handlers/
│   ├── titlebar.handlers.ts          Minimize, maximize, close
│   ├── auth.handlers.ts              Login, create identity, logout
│   ├── buddy.handlers.ts             Double-click, context menu, add friend
│   ├── chat.handlers.ts              Send message, key handling
│   ├── community.handlers.ts         Create, join, channel actions
│   ├── voice.handlers.ts             Join/leave, mute/deafen
│   └── settings.handlers.ts          Preference changes
├── styles/
│   ├── global.css                    Global Tailwind styles
│   ├── animations.css                Keyframe animations
│   ├── scrollbar.css                 Custom scrollbar styling
│   └── xfire-theme.css               Xfire-inspired theme variables
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
| `/chat?peer={key}` | `ChatWindow` |
| `/community?id={id}` | `CommunityWindow` |
| `/settings` | `SettingsWindow` |
| `/profile?key={key}` | `ProfileWindow` |

The fallback route renders `LoginWindow`.

## Stores

Stores use SolidJS `createStore()` for reactive state. Each store is populated
by event listeners registered in `channels.ts` and hydrated on login via
`hydrate.ts`.

### auth.store.ts

```
AuthState {
    isLoggedIn: boolean
    publicKey: string | null
    displayName: string | null
    avatarUrl: string | null
    status: 'online' | 'away' | 'busy' | 'offline'
    statusMessage: string | null
    gameInfo: GameStatus | null
}
```

### friends.store.ts

```
FriendsState {
    friends: Record<publicKey, Friend>
    pendingRequests: PendingRequest[]
    contextMenu: ContextMenuState | null
    showAddFriend: boolean
    showNewChat: boolean
}

Friend {
    publicKey: string
    displayName: string
    nickname: string | null
    status: UserStatus
    statusMessage: string | null
    gameInfo: GameInfo | null
    group: string
    unreadCount: number
    lastSeenAt: number | null
    voiceChannel: string | null
}
```

### chat.store.ts

```
ChatState {
    conversations: Record<peerId, Conversation>
    activeConversation: string | null
}

Conversation {
    messages: Message[]
    isTyping: boolean
    unreadCount: number
}
```

### community.store.ts

```
CommunityState {
    communities: Record<id, Community>
    activeChannel: string | null
}
```

### voice.store.ts

```
VoiceState {
    isConnected: boolean
    channelId: string | null
    isMuted: boolean
    isDeafened: boolean
    participants: VoiceParticipant[]
}
```

## IPC Layer

### commands.ts

Typed wrappers around `invoke()` for all Tauri commands. Each function maps
directly to a `#[tauri::command]` in the Rust backend.

### channels.ts

Event subscriptions using `listen()` from `@tauri-apps/api/event`. Subscribes
to four event channels:

| Event Name | Enum Type | Updates |
|------------|-----------|---------|
| `chat-event` | `ChatEvent` | Messages, typing, friend requests |
| `presence-event` | `PresenceEvent` | Online/offline, status, game changes |
| `voice-event` | `VoiceEvent` | Join/leave, speaking, mute state |
| `notification-event` | `NotificationEvent` | System alerts, update notifications |

In E2E testing mode (`VITE_E2E=true`), `safeListen()` is a no-op because the
Tauri event system is not available in a browser context.

### invoke.ts

Conditional invoke wrapper. In production, delegates to
`@tauri-apps/api/core` invoke. In E2E mode (`VITE_E2E=true`), sends HTTP POST
to the E2E bridge server at `http://127.0.0.1:3001/invoke`. Window navigation
commands (e.g., `show_buddy_list`) trigger browser `location.href` changes in
E2E mode.

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
