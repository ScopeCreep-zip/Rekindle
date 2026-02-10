# Tauri 2 Patterns for Rekindle

## Custom Window Chrome (Classic Xfire Skin)

Xfire was famous for its skinned UI. Tauri 2 supports this with frameless transparent windows.

### Window Configuration

```jsonc
// tauri.conf.json
{
  "app": {
    "windows": [{
      "label": "buddy-list",
      "decorations": false,   // Remove OS titlebar
      "transparent": true,    // Allow CSS to control background
      "shadow": true,         // Keep drop shadow (Win11 gets native rounded corners)
      "width": 280,
      "height": 600
    }]
  }
}
```

### Custom Titlebar

```html
<!-- data-tauri-drag-region enables native window dragging -->
<div data-tauri-drag-region class="xfire-titlebar">
  <img src="/icons/rekindle-logo.png" class="titlebar-icon" />
  <span class="titlebar-text">Rekindle</span>
  <div class="titlebar-buttons">
    <button id="btn-minimize" class="xfire-btn">_</button>
    <button id="btn-close" class="xfire-btn">X</button>
  </div>
</div>
```

```typescript
import { getCurrentWindow } from '@tauri-apps/api/window';

const appWindow = getCurrentWindow();
document.getElementById('btn-minimize')!.addEventListener('click', () => appWindow.minimize());
document.getElementById('btn-close')!.addEventListener('click', () => appWindow.hide()); // hide to tray
```

### Required Permissions

```jsonc
// capabilities/default.json
{
  "identifier": "default",
  "windows": ["buddy-list", "chat-*", "login", "settings"],
  "permissions": [
    "core:default",
    "core:window:default",
    "core:window:allow-start-dragging",
    "core:window:allow-close",
    "core:window:allow-minimize",
    "core:window:allow-hide",
    "core:window:allow-show",
    "core:window:allow-set-focus",
    "core:webview:allow-create-webview-window",
    "notification:default",
    "store:default",
    "sql:default",
    "single-instance:default",
    "window-state:default",
    "global-shortcut:default",
    "deep-link:default"
  ]
}
```

### Xfire Classic Theme CSS

```css
/* xfire-skin.css - Classic Xfire dark blue theme */
:root {
  --xfire-bg-dark: #1a1a2e;
  --xfire-bg-panel: #16213e;
  --xfire-bg-input: #0f3460;
  --xfire-accent: #e94560;
  --xfire-text: #e0e0e0;
  --xfire-text-dim: #8888aa;
  --xfire-online: #53d769;
  --xfire-away: #ffcc00;
  --xfire-ingame: #4fc3f7;
  --xfire-border: #2a2a4a;
  --xfire-titlebar-height: 28px;
  --xfire-font: 'Segoe UI', 'Tahoma', sans-serif;
  --xfire-font-size: 12px;
}

html, body {
  margin: 0;
  padding: 0;
  background: transparent;
  font-family: var(--xfire-font);
  font-size: var(--xfire-font-size);
  color: var(--xfire-text);
  overflow: hidden;
  user-select: none;
}

.app-frame {
  background: var(--xfire-bg-dark);
  border: 1px solid var(--xfire-border);
  border-radius: 4px;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.xfire-titlebar {
  height: var(--xfire-titlebar-height);
  background: linear-gradient(to right, #0f1a3a, #1a2a5e);
  display: flex;
  align-items: center;
  padding: 0 8px;
  gap: 6px;
  flex-shrink: 0;
}

.buddy-list {
  flex: 1;
  overflow-y: auto;
  padding: 4px 0;
}

.buddy-group-header {
  padding: 4px 12px;
  font-weight: bold;
  color: var(--xfire-text-dim);
  font-size: 11px;
  text-transform: uppercase;
}

.buddy-item {
  display: flex;
  align-items: center;
  padding: 3px 12px;
  gap: 8px;
  cursor: pointer;
}

.buddy-item:hover {
  background: rgba(255, 255, 255, 0.05);
}

.buddy-status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
}

.buddy-status-dot.online { background: var(--xfire-online); }
.buddy-status-dot.away { background: var(--xfire-away); }
.buddy-status-dot.ingame { background: var(--xfire-ingame); }
.buddy-status-dot.offline { background: var(--xfire-text-dim); }

.status-bar {
  height: 28px;
  background: var(--xfire-bg-panel);
  border-top: 1px solid var(--xfire-border);
  display: flex;
  align-items: center;
  padding: 0 8px;
  font-size: 11px;
  color: var(--xfire-text-dim);
  flex-shrink: 0;
}
```

## System Tray

Essential for an IM client - minimize to tray, status controls, unread badge.

```rust
use tauri::tray::{TrayIconBuilder, MouseButton, TrayIconEvent};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};

pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Status submenu
    let online = MenuItem::with_id(app, "status-online", "Online", true, None::<&str>)?;
    let away = MenuItem::with_id(app, "status-away", "Away", true, None::<&str>)?;
    let busy = MenuItem::with_id(app, "status-busy", "Busy", true, None::<&str>)?;
    let status_menu = Submenu::with_items(app, "Status", true, &[&online, &away, &busy])?;

    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Rekindle", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&status_menu, &sep, &quit])?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Rekindle")
        .menu(&menu)
        .menu_on_left_click(false)
        .on_menu_event(|app, event| {
            match event.id.as_ref() {
                "quit" => app.exit(0),
                id if id.starts_with("status-") => {
                    // Emit status change to frontend
                    let _ = app.emit("status-change-requested", id);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
                let window = tray.app_handle().get_webview_window("buddy-list").unwrap();
                let _ = window.show();
                let _ = window.set_focus();
            }
        })
        .build(app)?;

    Ok(())
}
```

## Multi-Window Chat (Classic Xfire Style)

Xfire opened a separate window for each conversation.

```rust
use tauri::{WebviewUrl, WebviewWindowBuilder};

#[tauri::command]
async fn open_chat_window(
    app: tauri::AppHandle,
    userid: u32,
    nickname: String,
) -> Result<(), String> {
    let label = format!("chat-{}", userid);

    // Check if window already exists
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    // Create new chat window
    let url = WebviewUrl::App(format!("/chat?userid={}&nick={}", userid, nickname).into());
    WebviewWindowBuilder::new(&app, &label, url)
        .title(format!("Chat - {}", nickname))
        .inner_size(400.0, 500.0)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .build()
        .map_err(|e| e.to_string())?;

    Ok(())
}
```

## Plugin Registration Order

Order matters. Single-instance must be first.

```rust
// lib.rs
pub fn run() {
    tauri::Builder::default()
        // MUST be first
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = app.get_webview_window("buddy-list")
                .map(|w| { w.show().ok(); w.set_focus().ok(); });
        }))
        // Then other plugins
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_stronghold::Builder::new(|pass| {
            // Key derivation from password
            todo!("implement key derivation")
        }).build())
        .plugin(tauri_plugin_sql::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent, None
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            setup_tray(app)?;
            Ok(())
        })
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            login, logout,
            send_message, subscribe_chat,
            add_friend, remove_friend, accept_friend, get_friends,
            set_status, set_nickname,
            open_chat_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Rekindle");
}
```

## Notifications

```typescript
import { sendNotification, isPermissionGranted, requestPermission }
  from '@tauri-apps/plugin-notification';

async function notifyNewMessage(from: string, body: string) {
  let permitted = await isPermissionGranted();
  if (!permitted) {
    const result = await requestPermission();
    permitted = result === 'granted';
  }
  if (permitted) {
    sendNotification({ title: from, body, icon: 'icons/message.png' });
  }
}
```

## Platform-Specific Notes

| Platform  | Window Chrome                                           |
|-----------|---------------------------------------------------------|
| Windows 11 | `shadow: true` gives native rounded corners for free   |
| Windows 10 | Use CSS `border-radius` + transparent; may have artifacts |
| macOS     | Consider `tauri-plugin-decorum` for clean corners       |
| Linux     | Depends on compositor; Wayland generally works well     |
