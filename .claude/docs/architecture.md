# Rekindle - Tauri 2 Architecture

## Project Structure

```
Cargo.toml                          # Workspace root
package.json                        # Frontend package manifest
vite.config.ts                      # Vite config
tsconfig.json                       # TypeScript config
flake.nix                           # Nix flake (imports Konductor)
index.html                          # Frontend entry point

src/                                # Frontend source
  main.ts                           # Frontend entry
  App.tsx                           # Root component
  components/
    titlebar/                       # Custom Xfire-style titlebar
      Titlebar.tsx
      Titlebar.css
    buddy-list/                     # Friends list (main window)
      BuddyList.tsx
      BuddyGroup.tsx
      BuddyItem.tsx
    chat/                           # Chat window components
      ChatWindow.tsx
      MessageList.tsx
      MessageInput.tsx
      TypingIndicator.tsx
    status/                         # Status selector
      StatusPicker.tsx
    login/                          # Login screen
      LoginForm.tsx
    tray/                           # System tray menu
      TrayMenu.tsx
  hooks/                            # Frontend hooks for Tauri IPC
    useChat.ts                      # Chat channel subscription
    usePresence.ts                  # Friend presence updates
    useFriends.ts                   # Friend list state
    useAuth.ts                      # Login/logout
  styles/
    xfire-skin.css                  # Classic Xfire skin (1:1 recreation)
    variables.css                   # Theme colors and sizing
  assets/
    icons/                          # Xfire-style icons
    sounds/                         # Notification sounds

src-tauri/                          # Tauri Rust backend
  Cargo.toml
  build.rs
  tauri.conf.json                   # Tauri config (frameless, transparent)
  capabilities/
    default.json                    # Base permissions
    chat-window.json                # Chat window permissions
  icons/                            # App icons
  src/
    main.rs                         # Desktop entry point
    lib.rs                          # Shared: commands, plugins, state
    commands/
      mod.rs
      auth.rs                       # login, logout, register
      chat.rs                       # send_message, subscribe_chat
      friends.rs                    # add_friend, remove_friend, accept_request
      status.rs                     # set_status, set_nickname
      game.rs                       # get_game_status, game_info
    state.rs                        # AppState (connection, session, friends)
    windows.rs                      # Window management (buddy list, chat windows)
    tray.rs                         # System tray setup and event handling

crates/
  rekindle-protocol/                # Pure Rust protocol library (NO Tauri dep)
    Cargo.toml
    src/
      lib.rs
      packet.rs                     # Packet header + framing
      attribute.rs                  # Attribute type system
      codec.rs                      # tokio_util Codec (Encoder + Decoder)
      crypto.rs                     # SHA-1 double-hash auth
      error.rs
      messages/
        mod.rs
        login.rs                    # Packets 1, 3, 18, 128-130
        chat.rs                     # Packets 2, 133 + peermsg subtypes
        friends.rs                  # Packets 5-9, 131, 132, 136-143
        game.rs                     # Packets 4, 135
        status.rs                   # Packets 11, 141, 142
        system.rs                   # Packets 12, 14, 16, 134
        groups.rs                   # Packets 154, 158
    tests/
      packets.rs                    # Known byte sequence round-trip tests
      auth.rs                       # Hash verification tests

  rekindle-game-detect/             # Game detection crate
    Cargo.toml
    src/
      lib.rs
      process.rs                    # Cross-platform process enumeration
      database.rs                   # Game ID <-> process name mapping
      platform/
        windows.rs                  # Windows-specific (CreateToolhelp32Snapshot)
        macos.rs                    # macOS process list
        linux.rs                    # /proc enumeration
```

## Data Flow

```
┌─────────────────────────────────────────────────────┐
│  Frontend (Webview)                                 │
│  ┌─────────┐  ┌──────────┐  ┌───────────────────┐  │
│  │ Buddy   │  │ Chat     │  │ Login / Status    │  │
│  │ List    │  │ Windows  │  │ Components        │  │
│  └────┬────┘  └────┬─────┘  └────────┬──────────┘  │
│       │            │                  │             │
│  ┌────┴────────────┴──────────────────┴──────────┐  │
│  │        Tauri IPC (Commands / Channels)        │  │
│  └───────────────────┬───────────────────────────┘  │
└──────────────────────┼──────────────────────────────┘
                       │
┌──────────────────────┼──────────────────────────────┐
│  Rust Backend        │                              │
│  ┌───────────────────┴───────────────────────────┐  │
│  │           src-tauri (Tauri Commands)           │  │
│  │  commands/auth  commands/chat  commands/friends│  │
│  └──────────┬──────────────┬─────────────────────┘  │
│             │              │                        │
│  ┌──────────┴──────┐ ┌────┴──────────────────┐     │
│  │ rekindle-       │ │ rekindle-game-detect   │     │
│  │ protocol        │ │ (process scanning)     │     │
│  │ (TCP:25999)     │ └───────────────────────┘     │
│  └────────┬────────┘                               │
└───────────┼─────────────────────────────────────────┘
            │
     ┌──────┴──────┐
     │ Xfire Server │
     │ TCP:25999    │
     └─────────────┘
```

## IPC Design

### Commands (Frontend -> Rust)

```rust
// auth.rs
#[tauri::command]
async fn login(username: String, password: String,
               state: tauri::State<'_, AppState>) -> Result<LoginResult, String>;

#[tauri::command]
async fn logout(state: tauri::State<'_, AppState>) -> Result<(), String>;

// chat.rs
#[tauri::command]
async fn send_message(to_sid: [u8; 16], body: String,
                      state: tauri::State<'_, AppState>) -> Result<(), String>;

#[tauri::command]
async fn subscribe_chat(on_event: Channel<ChatEvent>,
                        state: tauri::State<'_, AppState>) -> Result<(), String>;

// friends.rs
#[tauri::command]
async fn add_friend(username: String, message: String,
                    state: tauri::State<'_, AppState>) -> Result<(), String>;

#[tauri::command]
async fn get_friends(state: tauri::State<'_, AppState>) -> Result<Vec<Friend>, String>;

// status.rs
#[tauri::command]
async fn set_status(status: u32, message: String,
                    state: tauri::State<'_, AppState>) -> Result<(), String>;
```

### Channels (Rust -> Frontend, streaming)

```rust
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
enum ChatEvent {
    MessageReceived { from: String, sid: [u8; 16], body: String, timestamp: u64 },
    TypingIndicator { sid: [u8; 16], typing: bool },
    PresenceUpdate { userid: u32, status: String, game: Option<GameInfo> },
    FriendOnline { userid: u32, sid: [u8; 16] },
    FriendOffline { userid: u32 },
    FriendRequest { from: String, message: String },
}
```

### Events (window-to-window)

```rust
// Notify all windows of theme change
app.emit("theme-changed", &new_theme)?;

// Notify buddy list to refresh after friend action in chat window
app.emit_to("buddy-list", "friends-updated", ())?;
```

## Window Management

Classic Xfire used separate windows. Replicate this:

| Window         | Label            | Size       | Notes                              |
|----------------|------------------|------------|------------------------------------|
| Buddy List     | `buddy-list`     | 280x600    | Main window, always open, tray min |
| Chat           | `chat-{userid}`  | 400x500    | One per conversation, dynamic      |
| Login          | `login`          | 300x400    | Shown before auth, replaced by buddy list |
| Settings       | `settings`       | 450x500    | Modal-like, single instance        |
| Profile        | `profile-{uid}`  | 350x450    | Friend profile viewer              |

All windows: `decorations: false`, `transparent: true` for custom Xfire skin chrome.

## Tauri Configuration Highlights

```jsonc
// tauri.conf.json (key fields)
{
  "productName": "Rekindle",
  "identifier": "com.rekindle.app",
  "app": {
    "windows": [
      {
        "label": "buddy-list",
        "title": "Rekindle",
        "width": 280,
        "height": 600,
        "decorations": false,
        "transparent": true,
        "resizable": true,
        "shadow": true
      }
    ],
    "trayIcon": {
      "iconPath": "icons/tray.png",
      "tooltip": "Rekindle"
    }
  },
  "plugins": {
    "sql": { "preload": { "db": "sqlite:rekindle.db" } }
  }
}
```

## State Management

```rust
// state.rs
pub struct AppState {
    pub connection: Arc<Mutex<Option<XfireConnection>>>,
    pub session: Arc<Mutex<Option<Session>>>,
    pub friends: Arc<RwLock<FriendList>>,
    pub chat_channels: Arc<RwLock<HashMap<u32, Channel<ChatEvent>>>>,
}

pub struct Session {
    pub userid: u32,
    pub sid: [u8; 16],
    pub username: String,
    pub nickname: String,
}
```

## Dependencies

```toml
# Workspace Cargo.toml
[workspace]
members = ["src-tauri", "crates/rekindle-protocol", "crates/rekindle-game-detect"]

# crates/rekindle-protocol/Cargo.toml
[dependencies]
bytes = "1"
nom = "7"
sha1 = "0.10"
tokio = { version = "1", features = ["net", "io-util", "sync", "macros"] }
tokio-util = { version = "0.7", features = ["codec"] }
serde = { version = "1", features = ["derive"] }
thiserror = "2"
tracing = "0.1"

# src-tauri/Cargo.toml
[dependencies]
rekindle-protocol = { path = "../crates/rekindle-protocol" }
rekindle-game-detect = { path = "../crates/rekindle-game-detect" }
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-notification = "2"
tauri-plugin-window-state = "2"
tauri-plugin-store = "2"
tauri-plugin-stronghold = "2"
tauri-plugin-sql = { version = "2", features = ["sqlite"] }
tauri-plugin-single-instance = "2"
tauri-plugin-updater = "2"
tauri-plugin-process = "2"
tauri-plugin-deep-link = "2"
tauri-plugin-autostart = "2"
tauri-plugin-global-shortcut = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = "0.3"
```

## Testing Strategy

- **Protocol**: Known byte sequences -> parsed packets (and reverse). Fuzz with `cargo-fuzz`.
- **Integration**: Run against PFire server emulator for end-to-end validation.
- **Frontend E2E**: Playwright (provided by Konductor `frontend` shell).
- **Visual regression**: Screenshot comparison against classic Xfire UI captures.
