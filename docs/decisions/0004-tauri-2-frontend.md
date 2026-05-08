# 0004 — Use Tauri 2 + SolidJS as the desktop frontend

- **Status:** Accepted
- **Date:** 2026-04

## Context and problem statement

Rekindle ships a desktop app that needs:

- **Native Windows / macOS / Linux distribution** with appropriate
  packaging (NSIS / DMG / AppImage / deb).
- **A frameless, transparent, custom-skinned UI** matching the
  classic Xfire visual identity.
- **A Rust backend** because the protocol stack (Veilid, Signal,
  CRDT merge, voice pipeline) lives in Rust.
- **A small distribution size** to keep installs friction-free.
- **Multi-window support** (login, buddy list, per-conversation chat,
  per-DM, per-community, settings, profile) with custom titlebars.
- **Compatibility with the OS keyring, system tray, deep links,
  notifications, autostart, and global hotkeys** without writing
  per-platform glue.

## Decision drivers

- **Rust-native backend.** No marshalling protocol overhead, no
  language barrier between the protocol layer and the IPC bridge.
- **Small bundle size.** Cross-platform distribution favours small
  artifacts.
- **Frameless transparent windows** that look identical across OSes.
- **Custom skinning** in standard web technology so designers can
  iterate quickly.
- **Active maintenance.** The frontend toolchain must be alive in 2026
  and beyond.

## Considered options

### Option A — Tauri 2 + SolidJS (selected)

Rust backend with WebView frontend. SolidJS for a React-like component
model with fine-grained reactivity (no virtual DOM diff).

### Option B — Electron + React/Vue

The dominant cross-platform desktop framework. Bundles Chromium.

### Option C — Native per-OS with bridges

WinUI / SwiftUI / GTK with Rust core via FFI.

### Option D — egui (Rust-native immediate-mode)

Pure-Rust GUI framework.

### Option E — Slint or Iced (Rust-native retained-mode)

Pure-Rust GUI frameworks targeting cross-platform.

### Option F — Flutter

Dart-based cross-platform framework with its own renderer.

## Decision outcome

**Chose Tauri 2 + SolidJS.**

Tauri's Rust backend is a natural fit for a project with a heavy
Rust core. The WebView frontend gives us standard web tooling
(Tailwind 4, Vite, Playwright E2E) without bundling Chromium —
distribution sizes are dramatically smaller than Electron (typical
Tauri app: ~10–20 MB vs Electron's ~80–150 MB).

SolidJS over React: SolidJS's fine-grained reactivity matches the
shape of our state — many small reactive stores, frequent updates,
no need for a virtual DOM diff. The component model is React-like
enough that contributors with React experience are productive
immediately.

Tauri 2's plugin ecosystem covers our needs:
`single-instance` (prevent multiple instances), `notification`,
`store` (preferences), `process`, `deep-link` (`rekindle://`),
`autostart`, `global-shortcut`. We replaced `stronghold` with
direct `iota_stronghold` and `sql` with direct `rusqlite` to keep
control of the dependency graph.

Frameless transparent windows fall out of `decorations: false`
+ `transparent: true` in `tauri.conf.json` and the
`WebviewWindowBuilder`. The custom titlebar is a SolidJS component
with `data-tauri-drag-region`.

## Consequences

**Positive.**

- Rust-native backend — the protocol stack imports cleanly.
- Small distribution sizes (10–20 MB).
- Tauri's command/event IPC is type-safe with `serde` —
  command input/output structures match across the language
  boundary by convention.
- WebView is the user's system WebView (WebKit on macOS,
  WebView2 on Windows, WebKitGTK on Linux) — security
  patches arrive via OS updates rather than waiting for a
  Chromium-bundle refresh.
- SolidJS's fine-grained reactivity gives sub-millisecond UI updates
  even with many open windows watching presence streams.

**Negative.**

- **Three different WebView engines** (WebKit, WebView2, WebKitGTK)
  means cross-platform CSS testing matters. Modern features (CSS
  containment, `clip-path`, custom scrollbars) need fallbacks per
  engine.
- **Tauri's macOS event-loop quirks** forced us to drop the
  `window-state` plugin — see CLAUDE.md.
- **Veilid's deeply-nested future types** require
  `#![recursion_limit = "512"]` in any crate that holds long-lived
  futures over Tauri commands. This is a small ergonomic tax.
- **Argon2 perf** in debug mode is slow because of `iota_stronghold`'s
  internal use of `rust-argon2`. We override with
  `[profile.dev.package.rust-argon2] opt-level = 3`.
- **Less mature than Electron** — fewer Stack Overflow answers,
  smaller plugin ecosystem. We've found this to be less of a problem
  than expected because the cross-platform OS-integration plugins we
  need exist.

**Boundaries.**

- All business logic in the Rust backend; the SolidJS frontend is
  thin and renders state. No business logic in stores or components.
- IPC types use `#[serde(rename_all = "camelCase")]` on the Rust side
  and matching TypeScript types on the JS side.
- Tailwind 4 styles live in `src/styles/` only. No inline classes in
  components.

## Pros and cons of the options

### Tauri 2 + SolidJS (chosen)

- **+** Rust-native backend.
- **+** Small bundle size.
- **+** Active project, modern (v2 released 2024).
- **+** Modern web tooling for the frontend.
- **−** Cross-WebView CSS testing.
- **−** Smaller community vs Electron.

### Electron + React/Vue

- **+** Largest desktop-app community.
- **+** One Chromium engine across all platforms — predictable.
- **−** 80–150 MB distribution.
- **−** No native Rust path; backend is Node.js or a separate
  process.
- **−** Chromium update cycle is your problem.

### Native per-OS with bridges

- **+** Best possible OS integration.
- **−** 3× the GUI code.
- **−** No standard skinning toolchain.
- **−** Requires platform expertise we'd have to acquire and
  maintain.

### egui

- **+** Pure Rust.
- **+** Tiny bundle.
- **−** Immediate-mode GUI is a poor fit for a window-rich, IM-style
  app with many subscribed reactive stores.
- **−** Custom skinning is harder than CSS.

### Slint / Iced

- **+** Pure Rust, retained-mode.
- **−** Slint is paid for commercial use beyond a license tier
  threshold; license complexity we don't want.
- **−** Iced is younger, smaller community.
- **−** Custom skinning is more limited than CSS.

### Flutter

- **+** Cross-platform consistency.
- **+** Mature.
- **−** Dart adds a third language to the project.
- **−** Custom-painted widgets — CSS-style theming requires a fight.

## More information

- [Tauri 2 documentation](https://v2.tauri.app/)
- [SolidJS documentation](https://www.solidjs.com/)
- [`../architecture/frontend.md`](../architecture/frontend.md)
- [`../architecture/tauri-backend.md`](../architecture/tauri-backend.md)
- [`../architecture/ui-skin.md`](../architecture/ui-skin.md)
