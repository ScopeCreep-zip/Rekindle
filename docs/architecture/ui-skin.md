# UI and Visual Skin

Rekindle is an explicit **1:1 visual recreation of the classic Xfire
client**. Every window is frameless, transparent at the edges, and
skinned to match Xfire's signature dark-blue gaming-IM aesthetic. This
is a load-bearing design goal, not decoration: the project's identity
is "the Xfire experience, on a P2P substrate."

The implementation lives in:

- **`src-tauri/src/windows.rs`** — Tauri `WebviewWindowBuilder` setup.
- **`src-tauri/tauri.conf.json`** — declared windows + their defaults.
- **`src/main.tsx`** — pathname-based router that picks the window
  component.
- **`src/windows/`** — one component per top-level window.
- **`src/components/titlebar/Titlebar.tsx`** — the custom drag region
  and window controls.
- **`src/styles/`** — the Tailwind 4 theme, animation library, and
  custom scrollbars.
- **`legacy/unpacked/skins/`** — the original Symbiosis skin (460 GIF
  assets + XML layouts + `Themes.xml`) extracted from the Xfire
  installer for visual reference.

## Frameless transparent windows

Every window is created with:

```rust
WebviewWindowBuilder::new(app, label, url)
    .decorations(false)
    .transparent(true)
    .resizable(true)
    .shadow(true)
    .build()
```

`decorations: false` removes the OS title bar; `transparent: true`
lets the window's outer edges blend with whatever is behind them so
the rounded-corner skin can render correctly. The custom `Titlebar`
component fills the role of the OS title bar — it provides the drag
region (via Tauri's `data-tauri-drag-region`), the window-control
buttons (minimise, maximise, close), and the per-window title text in
Xfire's typeface.

This pattern repeats in every window: there is a single `Titlebar` at
the top, a `<main class="window-main">` wrapper that takes the
flex-1 slot, and a footer (status bar, message input, action bar).

## Window catalogue

Only the **login** window is declared statically in `tauri.conf.json`.
Every other window is created at runtime by helpers in
`src-tauri/src/windows.rs`. Each window carries a unique `label` and
URL path; the SolidJS `Switch` in `main.tsx` reads
`window.location.pathname` and renders the matching component.

| Window | Label | Path | Notes |
|--------|-------|------|-------|
| Login | `login` | `/login` | 380 × 480, declared in `tauri.conf.json` |
| Buddy list | `buddy-list` | `/buddy-list` | Narrow vertical (320 × 650), hides to tray on close |
| Chat | `chat-{pubkey}` | `/chat?peer={key}` | One per 1:1 friend conversation |
| DM | `dm-{record-key}` | `/dm?record={key}` | One per DM / group DM |
| Community | `community-{id}` | `/community?id={id}` | One per joined community |
| Settings | `settings` | `/settings` | Single instance |
| Profile | `profile-{key}` | `/profile?key={key}` | One per peer profile view |

Closing the **login** window when no buddy list is visible exits the
app; closing the **buddy list** hides it to the system tray. This
matches the Xfire window-management feel — the tray icon is the app's
home base, not the buddy list.

The narrow vertical buddy list (320 px wide) is deliberate. Xfire's
shape is unmistakable, and copying its aspect ratio is part of the
nostalgia goal.

## Tailwind 4 theme

`src/styles/global.css` declares the Xfire palette as Tailwind theme
tokens. Every colour used by the app is one of these tokens — there
are **no inline hex codes** in components. Component styles compose
the tokens via `@apply` in `xfire-theme.css`.

```css
@theme {
  /* Surfaces */
  --color-xfire-bg-dark:        #0a0a0a;   /* outer chrome */
  --color-xfire-bg-panel:       #1e1e1e;   /* panel fill */
  --color-xfire-bg-input:       #1e1e1e;
  --color-xfire-bg-status:      #181818;
  --color-xfire-bg-tooltip:     #282828;
  --color-xfire-bg:             #0a0a0a;
  --color-xfire-bg-primary:     #141414;
  --color-xfire-bg-secondary:   #1a1a1a;
  --color-xfire-bg-tertiary:    #252525;
  --color-xfire-border:         #2a2a2a;

  /* Text */
  --color-xfire-text-primary:   #e7e7e7;
  --color-xfire-text-secondary: #a0a0a0;
  --color-xfire-text:           #e7e7e7;
  --color-xfire-text-dim:       #8fb9d7;
  --color-xfire-text-clan:      #1ba1fd;
  --color-xfire-text-status:    #969696;
  --color-xfire-text-timestamp: #a0a0a0;

  /* Accent */
  --color-xfire-accent:         #177cc1;   /* The Xfire blue */
  --color-xfire-link:           #5fa8d3;
  --color-xfire-focus-outline:  #5fa8d3;

  /* Status indicators */
  --color-xfire-online:         #53d769;
  --color-xfire-away:           #ffcc00;
  --color-xfire-ingame:         #4fc3f7;
  --color-xfire-busy:           #ef4444;
  --color-xfire-offline:        #7d7d7d;
}
```

The blue (`#177cc1`) is the Xfire accent — used for selected items,
focus rings, links, and the in-game status indicator. The grey
hierarchy (`#0a → #14 → #1a → #1e → #25 → #2a`) gives subtle elevation
without breaking the dark-mode-first aesthetic.

## Style conventions

The project enforces three style rules in code review:

1. **Global styles only.** Tailwind utilities live in `src/styles/`,
   not as inline class strings on components. Components compose
   semantic class names (`.buddy-item`, `.message-bubble`,
   `.titlebar-button`) defined in `xfire-theme.css`. This keeps the
   component tree readable and lets the skin be swapped without
   touching component code.
2. **Window roots are flex columns.** Every top-level window is
   `display: flex; flex-direction: column;` with one `flex: 1` child
   (`.window-main`) holding the footer down. Wrappers introduced
   between the root and the flex-1 child must either preserve the flex
   contract or use `display: contents` so the footer stays bottom-stuck.
3. **No inline emojis.** The app uses Nerd Font glyphs (loaded from
   `src/assets/fonts/SymbolsNerdFontMono-Regular.woff2`) via the
   `.nf-icon` utility — emoji rendering is inconsistent across
   platforms and clashes with the period look. Reactions and message
   bodies *do* render Unicode emoji as a user-driven content type;
   the chrome itself does not.

These conventions live in [`../../CONTRIBUTING.md`](../../CONTRIBUTING.md)
under "Coding standards / TypeScript / SolidJS".

## Accessibility

The Xfire visual density is uncompromising; that puts pressure on
keyboard navigation and screen-reader support. The theme sheet
addresses this with three patterns from the architecture's a11y audit:

- **Global `:focus-visible` ring.** A 2-pixel accent ring on every
  interactive element when keyboard-focused; mouse focus is
  suppressed (`*:focus:not(:focus-visible) { outline: none; }`). The
  global rule is the safety net; component-specific overrides can
  refine the ring per element.
- **Skip link.** Each top-level window renders a `.skip-link` as its
  first focusable child. It is hidden via `clip-path: inset(100%)`
  (the modern alternative to the deprecated `left: -9999px` trick) so
  screen readers still announce it; on focus it pops to the top-left
  corner so keyboard users can bypass nav.
- **Live regions.** `.live-region-sr-only` (visually hidden via
  `clip-path`) wraps ARIA live regions for screen-reader-only
  announcements (incoming-message toasts, presence changes).

## Scrollbars and animations

Two satellite stylesheets handle the rest of the chrome:

- **`src/styles/scrollbar.css`** — narrow custom scrollbars that match
  the panel background, with the track invisible until hover. Falls
  back to native scrollbars on platforms that don't support the
  webkit/scrollbar-* properties.
- **`src/styles/animations.css`** — a small library of motion tokens
  (`@keyframes fade-in`, `@keyframes pulse`, status-dot transitions,
  modal slide-ins). Motion respects `prefers-reduced-motion: reduce`.

## Sound theme

The original Xfire shipped a default sound bank in
`legacy/unpacked/sounds/defaults.zip` and an alternate "classic" bank
in `classic.zip`. The intent is to surface these as user-selectable
notification sounds (per-channel via `notification_sound` in the
`notification_preferences` table — see [`data-layer.md`](data-layer.md)).
Soundboard custom expressions in communities are governed by the
expression system (architecture spec §18), separate from notification
sounds.

## System tray

`src-tauri/src/tray.rs` builds the system tray menu. The tray is the
app's primary "always-available" affordance — left-click toggles the
buddy list, right-click opens the status menu (Online / Away / Busy /
Offline + custom status). The tray icon shows the current presence via
a status dot rendered into the icon at runtime.

This means the buddy list can be hidden without losing access to the
status controls, matching Xfire's behaviour where the tray was the
home base.

## Reference assets

The original installer's contents are extracted to `legacy/unpacked/`
for visual reference. **Do not execute the binary;** the files there
are static assets only:

| Asset | Purpose |
|-------|---------|
| `skins/Symbiosis/*.gif` (460 files) | Per-element skin assets (window edges, buttons, status dots, gradients) |
| `skins/Symbiosis/*.xml` | Layout definitions linking assets to UI regions |
| `skins/Symbiosis/Themes.xml` | Colour palette overrides |
| `icons.dll` | 3,845 game icons indexed by Xfire game ID |
| `sounds/{defaults,classic}.zip` | Notification sound banks |
| `templates/*.html` | Original chat-bubble HTML templates |

`legacy/intended_architecture/10-ui-skin-system.md` walks through how
these assets map to current Tailwind classes. When implementing a new
chrome component, the workflow is: open the relevant Symbiosis GIF and
XML layout, identify the regions, port the colours into existing
theme tokens, and compose the component using `@apply` semantics.

## Where to look

| Concern | File |
|---------|------|
| Window builders (frameless + transparent) | `src-tauri/src/windows.rs` |
| Static login window declaration | `src-tauri/tauri.conf.json` |
| Pathname-based routing | `src/main.tsx` |
| Per-window components | `src/windows/{LoginWindow,BuddyListWindow,ChatWindow,DmWindow,CommunityWindow,SettingsWindow,ProfileWindow}.tsx` |
| Custom titlebar | `src/components/titlebar/Titlebar.tsx` |
| Theme tokens (Tailwind 4 `@theme`) | `src/styles/global.css` |
| Component skin classes | `src/styles/xfire-theme.css` |
| Animations | `src/styles/animations.css` |
| Custom scrollbars | `src/styles/scrollbar.css` |
| Nerd Font icon set | `src/assets/fonts/SymbolsNerdFontMono-Regular.woff2`, `src/icons.ts` |
| Reference skin assets | `legacy/unpacked/skins/Symbiosis/` |
| Reference notification sounds | `legacy/unpacked/sounds/` |
| System tray | `src-tauri/src/tray.rs` |

## Open work

- **Skin selector** — the architecture allows multiple skins (Symbiosis
  is the default), but the runtime selector + skin-pack format is not
  yet shipped. The `legacy/unpacked/skins/` extraction tooling is in
  place; the loader is pending.
- **High-DPI asset scaling** — the original Symbiosis GIFs are 1× pixel
  art. Components currently composite Tailwind tokens on top of a
  small subset of the original assets; full skin-fidelity at 2× / 3×
  needs vector or hi-res assets we don't have yet.
- **Touch-input refinements** for the dense Xfire interaction targets,
  pending mobile target.
