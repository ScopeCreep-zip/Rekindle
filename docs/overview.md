# Project Overview

## Vision

Rekindle is a faithful recreation of the classic Xfire gaming chat client, rebuilt from the ground
up as a modern Tauri 2 desktop application. The goal is to capture the exact look, feel, and
nostalgia of the original Xfire while using a modern, maintainable technology stack.

## What Was Xfire?

Xfire (2004–2015) was a gaming-focused instant messaging client that pioneered features now
considered standard:

- **Friends list** with online/away/in-game status
- **In-game overlay** for chatting without alt-tabbing
- **Game time tracking** (before Steam did it)
- **Game detection** via process scanning and network hooks
- **Game server browser** with join-game support
- **Screenshots and video** capture
- **Voice chat** (Xfire Pro-Voice)
- **Skinned UI** with a distinctive dark blue theme

At its peak, Xfire had 22+ million registered users and supported 3,000+ games. It was acquired by
Viacom for $102M in 2006, sold to Titan Gaming in 2010, and shut down in 2015.

## What Rekindle Aims To Be

### Phase 1: Protocol + Core Client
- Implement the Xfire binary protocol in pure Rust
- Build the Tauri 2 app shell with classic Xfire UI
- Login, friends list, 1-to-1 chat, status messages
- System tray with status controls
- Test against PFire server emulator

### Phase 2: Game Features
- Game detection (process scanning)
- Game status shown on buddy list (game name, server, map)
- Game time tracking

### Phase 3: Advanced Features
- P2P direct messaging (UDP hole punching)
- File transfer
- Group chat
- In-game overlay
- Voice chat

### Phase 4: Modern Additions
- Auto-update via Tauri updater
- `rekindle://` deep links
- Screenshot/clip sharing
- Profile pages

## Design Principles

1. **Nostalgia first** — if it looked a certain way in Xfire, it should look that way in Rekindle.
   Frameless windows, custom titlebar, compact buddy list, separate chat windows.

2. **Clean architecture** — the protocol library (`rekindle-protocol`) has zero knowledge of Tauri.
   It's a pure Rust crate that can be used in any context. Tauri wraps it with commands and
   channels.

3. **Cross-platform** — Windows, macOS, and Linux from day one. Game detection adapts per platform.

4. **Reproducible** — Konductor Nix flake ensures every developer and CI pipeline uses identical
   toolchains.

5. **Open** — MIT licensed. Reference existing open-source Xfire implementations for protocol
   behavior, but write clean-room code.

## Reverse Engineering Approach

The `xf1re_installer.exe` in the repository is from the Xf1re revival project (xf1re.com). It
serves as our primary reverse engineering target to:

1. Extract and catalog UI assets (skins, icons, layouts)
2. Verify documented protocol behavior against actual implementation
3. Understand game detection and overlay mechanisms
4. Identify any protocol extensions added by the revival project

All analysis is done statically. The binary is never executed.
