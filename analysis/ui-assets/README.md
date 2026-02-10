# UI Assets Catalog

All skin assets extracted from `skins/Symbiosis.zip` to `unpacked/skins_extracted/`.

## Asset Counts

| Category | Count | Location |
|----------|-------|----------|
| GIF images (skin) | 460 | `skins_extracted/Images/` |
| XML layout files | 35+ | `skins_extracted/*.xml`, `Components/`, `XIG/`, `XIG2/` |
| Game icons | 3,845 | `icons.dll` (PE resources, type ICONS) |
| Country flags | 239 | `Xfire.exe` (PE resources, type FLAGS) |
| Sound packs | 2 | `sounds/defaults.zip`, `sounds/classic.zip` |
| HTML templates | 50+ | `templates/`, `templates/infoview/` |
| CSS files | 12+ | `templates/infoview/styles/` + game-specific |
| JS files | 30+ | `templates/infoview/scripts/` + game-specific |

## Skin Image Categories

### Window Frames
- `Images/default/Frame1/` — main window frame (9-slice: top-left/middle/right, sides, bottom)
- `Images/default/Frame2/` — chat window frame
- All use 9-slice pattern for scalable window borders

### Buttons (largest category)
- `Images/default/Buttons/` — ~150 button images
- States: `norm`, `hover`, `down`, `disabled`
- Types: close, minimize, maximize, send, join, call, mute, tabs, etc.
- Large variants for prominent actions

### Common UI Elements
- `Images/default/Common/` — content containers, headers, menus, tabs, text fields, scrollbars
- 9-slice patterns for all resizable containers

### Scrollbars
- `Images/default/Scrollbars/` — complete custom scrollbar skin
- Arrow buttons (up/down/left/right × 4 states)
- Grippers (vertical/horizontal × 3 states)
- Gutters (vertical/horizontal + corner)
- "Grips" decoration on gripper thumb

### Icons
- `Images/default/Icons/` — status icons (chat, alert 1-9+, speaker states, etc.)

### In-Game Overlay (XIG)
- `Images/default/XIG/` — overlay frame, alerts, bar, music player, web browser, edit mode
- Separate overlay UI from desktop UI

## Color Palette (from Themes.xml)

Key Xfire colors:
```
Background:     RGBA(10,10,10)     — nearly black
List BG:        RGBA(10,10,10)     — buddy list background
Selection:      RGBA(23,124,193)   — Xfire blue selection
Friend text:    RGBA(231,231,231)  — light gray
Clan text:      RGBA(27,161,253)   — bright blue
FoF text:       RGBA(143,185,215)  — muted blue
Offline text:   RGBA(125,125,125)  — gray
Chat self:      RGBA(231,231,231)  — white
Chat other:     RGBA(143,185,215)  — blue-gray
Chat BG:        RGBA(30,30,30)     — dark gray
Link color:     RGBA(15,104,150)   — teal blue
Status bar BG:  RGBA(24,24,24)     — near black
Status text:    RGBA(150,150,150)  — medium gray
```

## Layout System

Xfire uses a **tile-based layout** system defined in XML:

```xml
<Tile Name="ButtonClose" X="5" Y="3" Z="25" JustX="Right" Component="ButtonClose" />
```

Properties:
- `X`, `Y` — position (absolute or relative: `OtherTile.Right+5`)
- `Z` — z-order layer
- `JustX`, `JustY` — alignment: Left, Center, Right / Top, Center, Bottom
- `ResizeX`, `ResizeY` — percentage of parent to fill (0-100)
- `IndentLeft/Right/Top/Bottom` — margins (can reference other tiles)
- `Component` — maps to a code-defined behavior

This is essentially a constraint-based layout similar to what CSS Flexbox or Grid provides.

## Key Insight for Rekindle

The Xfire skin system maps directly to CSS/HTML concepts:
- **Tiles** → CSS Grid/Flexbox positioned divs
- **9-slice frames** → CSS `border-image` or individual border elements
- **Button states** → CSS `:hover`, `:active`, `:disabled` pseudo-classes
- **Theme colors** → CSS custom properties / Tailwind theme
- **Z-ordering** → CSS `z-index`
- **Component binding** → Data attributes + JavaScript event handlers

We can recreate this faithfully in Tauri's webview with minimal abstraction overhead.
