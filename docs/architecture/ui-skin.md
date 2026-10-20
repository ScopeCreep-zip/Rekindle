# UI and Visual Skin

Rekindle is an explicit **1:1 visual recreation of the classic Xfire
client**. Every window is frameless, transparent at the edges, and
skinned to match Xfire's signature dark-blue gaming-IM aesthetic. This
is a load-bearing design goal, not decoration: the project's identity
is "the Xfire experience, on a P2P substrate."

The implementation lives in:

- **`src-tauri/src/windows/mod.rs`** — Tauri `WebviewWindowBuilder` setup
  for every window kind (`tauri.conf.json` declares none: `"windows": []`).
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

No window is declared statically (`tauri.conf.json` has
`"windows": []`). Every window, the login one included, is created at
runtime by helpers in `src-tauri/src/windows/mod.rs` (`open_login` runs
at setup). Each window carries a unique `label` and
URL path; the SolidJS `Switch` in `main.tsx` reads
`window.location.pathname` and renders the matching component.

| Window | Label | Path | Notes |
|--------|-------|------|-------|
| Login | `login` | `/login` | 380 × 480, opened by `open_login` at setup |
| Buddy list | `buddy-list` | `/buddy-list` | Narrow vertical (320 × 650), hides to tray on close |
| Chat | `chat-{pubkey}`, `chat-{record-key}` | `/chat?peer={key}`, `/chat?group={key}` | One per 1:1 friend conversation or group DM |
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
are **no inline hex codes** in components, and no colour literal in any
rule under `src/styles/`: no hex, no colour function and no named colour
such as `white` or `black` (`cargo xtask check-theme-literals`; the
system colours of the `forced-colors` rules are the user's palette and
stay). Literals live only in `global.css`'s token blocks: `@theme`, the
light-base values in `@layer xfire-defaults` and the reserved `--sec-*`
set in `@layer sec`. Component styles read the tokens with `var(--…)` in
the ordered parts under `src/styles/theme/*.css`.

```css
@theme {
  /* Surfaces */
  --color-xfire-bg-dark:        #0a0a0a;   /* outer chrome */
  --color-xfire-bg-panel:       #1e1e1e;   /* panel fill */
  --color-xfire-bg-input:       #1e1e1e;
  --color-xfire-bg-status:      #181818;
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
  --color-xfire-accent:         #167bc0;   /* The Xfire blue, as a fill */
  --color-xfire-accent-text:    #3491d8;   /* the same blue as text */
  --color-xfire-accent-fg:      #ffffff;   /* text on an accent fill */
  --color-xfire-danger:         #df3337;   /* destructive fills */
  --color-xfire-danger-text:    #ff5955;   /* errors and destructive text */
  --color-xfire-success-fill:   #00892d;   /* positive fills, white text */
  --color-xfire-link:           #5fa8d3;
  --color-xfire-focus-outline:  #5fa8d3;

  /* Status indicators */
  --color-xfire-online:         #53d769;
  --color-xfire-away:           #ffcc00;
  --color-xfire-ingame:         #4fc3f7;
  --color-xfire-online-text:    #53d769;   /* the status colours as text */
  --color-xfire-away-text:      #ffcc00;
  --color-xfire-ingame-text:    #4fc3f7;
  --color-xfire-busy:           #ef4444;
  --color-xfire-offline:        #7d7d7d;
  --color-xfire-offline-text:   #909090;   /* the offline grey as text */

  /* Overlays */
  --color-xfire-overlay:        #ffffff;   /* hover and raise, toward the text */
  --color-xfire-shade:          #000000;   /* recess, away from the text */
  --color-xfire-fill-shade:     #000000;   /* hover step of a white-text fill */
}
```

The blue is the Xfire accent. As a fill (`accent`, selected items,
buttons, badges) it carries white text at 4.54:1; as text on a ground it
is `accent-text`, which reads ≥ 4.5:1 on every grey (plan E7.7.0; the
original `#177cc1` failed both). Red, the offline grey and the other
status colours split the same way: the status colours stay for the dots,
and `danger-text`, `offline-text`, `online-text`, `away-text` and
`ingame-text` carry them as text (plan E7.7.0a). Hover and pressed
states lay `overlay` over the ground at a few percent (white on the dark
base, black on a light one), so one rule serves both bases. Every pair
the region map `src/styles/regions.toml` declares reads ≥ 4.5:1, the
status dots ≥ 3:1, on the dark base, and every pair whose text the light
base revalues reads the same on the light base, measured on white and its
tints until the skin registry (plan E7.7.1) gives the light base its own
grounds (`cargo xtask check-contrast-pairs`). This block is an excerpt:
the full set, with the per-base values and the reserved `--sec-*` tokens,
is `src/styles/global.css`. The grey
hierarchy (`#0a → #14 → #1a → #1e → #25 → #2a`) gives subtle elevation
without breaking the dark-mode-first aesthetic.

## Style conventions

The project enforces three style rules in code review:

1. **Global styles only.** Tailwind utilities live in `src/styles/`,
   not as inline class strings on components. Components compose
   semantic class names (`.buddy-item`, `.message-bubble`,
   `.titlebar-button`) defined in `src/styles/theme/*.css`. This keeps the
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

The user's settings beat any skin: the `forced-colors` rules, the
`prefers-contrast: more` overrides and the reduced-motion duration tokens
are in `@layer a11y`, above `skin` and `components`. Only the security
surfaces (`@layer sec`: the commit controls, the review card with the facts
and key fingerprint, the safety number, the pairing QR and code) sit above
them, and no skin can restyle those (`cargo xtask check-sec-surfaces`).

## Scrollbars and animations

Two satellite stylesheets handle the rest of the chrome:

- **`src/styles/scrollbar.css`** — narrow (8 px) custom WebKit
  scrollbars in `@layer components`: the track is always painted in
  `bg-dark`, the thumb is the offline grey at 40 % (60 % on hover, 80 %
  while dragged). An engine without the `::-webkit-scrollbar`
  pseudo-elements draws its native scrollbars.
- **`src/styles/animations.css`** — the typing-dot bounce, the
  new-message fade-in and the toast slide-in (`@keyframes typing-bounce`,
  `fade-in`, `toast-slide-in`). Every animation is declared under
  `prefers-reduced-motion: no-preference`, and the `--motion-duration-*`
  tokens are 0 ms unless the user allows motion.

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

When implementing a new chrome component, the workflow is: open the relevant Symbiosis GIF and
XML layout, identify the regions, port the colours into existing
theme tokens, and compose the component using `@apply` semantics.

## Where to look

| Concern | File |
|---------|------|
| Window builders (frameless + transparent), login included | `src-tauri/src/windows/mod.rs` |
| Pathname-based routing | `src/main.tsx` |
| Per-window components | `src/windows/{LoginWindow,BuddyListWindow,ChatWindow,CommunityWindow,SettingsWindow,ProfileWindow}.tsx` |
| Custom titlebar | `src/components/titlebar/Titlebar.tsx` |
| Theme tokens (Tailwind 4 `@theme`) | `src/styles/global.css` |
| Component skin classes | `src/styles/theme/*.css` (ordered parts; `16-security-surfaces.css` is `@layer sec`) |
| Region map (text grounds) | `src/styles/regions.toml` |
| Animations | `src/styles/animations.css` |
| Custom scrollbars | `src/styles/scrollbar.css` |
| Nerd Font icon set | `src/assets/fonts/SymbolsNerdFontMono-Regular.woff2`, `src/icons.ts` |
| Reference skin assets | `legacy/unpacked/skins/Symbiosis/` |
| Reference notification sounds | `legacy/unpacked/sounds/` |
| System tray | `src-tauri/src/tray.rs` |

## Open work

- **Skin selector** — the architecture allows multiple skins (plan E7.7),
  but the runtime selector and the `.rkskin` format are not yet shipped.
  Symbiosis becomes the default once E7.7.3 imports it; the Xfire
  importer that extracts it from `legacy/unpacked/skins/` is E7.7.3 too.
- **High-DPI asset scaling** — the original Symbiosis GIFs are 1× pixel
  art. Components currently composite Tailwind tokens on top of a
  small subset of the original assets; full skin-fidelity at 2× / 3×
  needs vector or hi-res assets we don't have yet.
- **Touch-input refinements** for the dense Xfire interaction targets,
  pending mobile target.
